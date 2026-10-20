//! The controller side of the host link: every connected device holds one
//! WebSocket to `GET /v1/ws/runtime-hosts/link`, greets with its host token,
//! and from then on answers the controller's calls and streams terminal
//! output. `HostLinkHub` is the registry a handler asks for a device.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{
    extract::{
        State,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use choruz_common::AppError;
use choruz_host_runtime::link::{ControllerFrame, DeviceFrame, LINK_PROTOCOL, LinkRequest};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::{Notify, mpsc, oneshot};

use crate::{ApiState, handlers_runtime_hosts};

/// Longest a device may take to answer one call; a session catalog scan on
/// a slow disk is the upper bound.
const CALL_TIMEOUT: Duration = Duration::from_secs(90);
const OUTBOUND_CAPACITY: usize = 256;
const TERMINAL_OUTPUT_CAPACITY: usize = 256;

#[derive(Clone, Default)]
pub(crate) struct HostLinkHub {
    links: Arc<StdMutex<HashMap<String, Arc<HostLink>>>>,
}

impl HostLinkHub {
    #[cfg(test)]
    pub(crate) fn decision_fixture(
        &self,
        host_id: &str,
    ) -> (Arc<HostLink>, mpsc::Receiver<String>) {
        let (outbound, received) = mpsc::channel(64);
        let link = Arc::new(HostLink {
            host_id: host_id.into(),
            host_name: "Decision fixture".into(),
            outbound,
            pending: StdMutex::new(HashMap::new()),
            terminals: StdMutex::new(HashMap::new()),
            closed: Notify::new(),
        });
        self.register(link.clone());
        (link, received)
    }
    pub(crate) fn get(&self, host_id: &str) -> Option<Arc<HostLink>> {
        self.links
            .lock()
            .expect("host links lock")
            .get(host_id)
            .cloned()
    }

    pub(crate) fn revoke(&self, host_id: &str) {
        if let Some(link) = self.links.lock().expect("host links lock").remove(host_id) {
            link.close();
        }
    }

    fn register(&self, link: Arc<HostLink>) -> Option<Arc<HostLink>> {
        self.links
            .lock()
            .expect("host links lock")
            .insert(link.host_id.clone(), link)
    }

    fn unregister(&self, link: &Arc<HostLink>) {
        let mut links = self.links.lock().expect("host links lock");
        if links
            .get(&link.host_id)
            .is_some_and(|current| Arc::ptr_eq(current, link))
        {
            links.remove(&link.host_id);
        }
    }
}

/// One connected device.
pub(crate) struct HostLink {
    pub(crate) host_id: String,
    pub(crate) host_name: String,
    outbound: mpsc::Sender<String>,
    pending: StdMutex<HashMap<String, oneshot::Sender<Result<Value, AppError>>>>,
    terminals: StdMutex<HashMap<String, TerminalSubscriber>>,
    closed: Notify,
}

#[derive(Clone)]
struct TerminalSubscriber {
    sender: mpsc::Sender<Vec<u8>>,
    resync: Arc<AtomicBool>,
}

impl HostLink {
    /// Send one request to the device and wait for its reply.
    pub(crate) async fn call(&self, request: LinkRequest) -> Result<Value, AppError> {
        let timeout = match &request {
            LinkRequest::Host {
                request: choruz_host_runtime::HostRequest::ReplayExperience { .. },
            } => Duration::from_secs(180),
            // Let the device's 120-second execution budget and session cleanup
            // finish before the controller declares the outcome unavailable.
            LinkRequest::Host {
                request: choruz_host_runtime::HostRequest::BrowserWorkflow { .. },
            } => Duration::from_secs(150),
            _ => CALL_TIMEOUT,
        };
        let id = choruz_common::new_id();
        let (reply_tx, reply_rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending calls lock")
            .insert(id.clone(), reply_tx);
        let frame = ControllerFrame::Call {
            id: id.clone(),
            request: Box::new(request),
        };
        let text = serde_json::to_string(&frame)
            .map_err(|error| AppError::Internal(format!("encode link call: {error}")))?;
        if self.outbound.send(text).await.is_err() {
            self.pending.lock().expect("pending calls lock").remove(&id);
            return Err(offline(&self.host_name));
        }
        match tokio::time::timeout(timeout, reply_rx).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => Err(offline(&self.host_name)),
            Err(_) => {
                self.pending.lock().expect("pending calls lock").remove(&id);
                Err(AppError::Internal(format!(
                    "runtime host {} did not respond in time",
                    self.host_name
                )))
            }
        }
    }

    /// Route the device's terminal output for `terminal_id` into a channel
    /// that closes when the device reports the terminal's exit or the link
    /// drops.
    pub(crate) fn subscribe_terminal(
        &self,
        terminal_id: &str,
    ) -> (mpsc::Receiver<Vec<u8>>, Arc<AtomicBool>) {
        let (tx, rx) = mpsc::channel(TERMINAL_OUTPUT_CAPACITY);
        let resync = Arc::new(AtomicBool::new(false));
        self.terminals
            .lock()
            .expect("terminal outputs lock")
            .insert(
                terminal_id.to_owned(),
                TerminalSubscriber {
                    sender: tx,
                    resync: resync.clone(),
                },
            );
        (rx, resync)
    }

    pub(crate) fn unsubscribe_terminal(&self, terminal_id: &str) {
        self.terminals
            .lock()
            .expect("terminal outputs lock")
            .remove(terminal_id);
    }

    pub(super) fn settle(&self, id: &str, outcome: Result<Value, AppError>) {
        if let Some(reply) = self.pending.lock().expect("pending calls lock").remove(id) {
            let _ = reply.send(outcome);
        }
    }

    fn close(&self) {
        self.closed.notify_one();
        let pending = std::mem::take(&mut *self.pending.lock().expect("pending calls lock"));
        for (_, reply) in pending {
            let _ = reply.send(Err(offline(&self.host_name)));
        }
        self.terminals
            .lock()
            .expect("terminal outputs lock")
            .clear();
    }
}

pub(crate) fn offline(host_name: &str) -> AppError {
    AppError::Conflict(format!("runtime host {host_name} is not connected"))
}

/// `GET /v1/ws/runtime-hosts/link`: a device's persistent link. The first
/// frame must be a `hello` carrying a valid host token; the HTTP layer has no
/// header to check because a relayed socket arrives with the paired
/// browser's identity, not the device's.
pub(crate) async fn websocket_host_link(
    ws: WebSocketUpgrade,
    State(state): State<ApiState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| serve_link(socket, state))
}

async fn serve_link(mut socket: WebSocket, state: ApiState) {
    let hello = tokio::time::timeout(Duration::from_secs(10), socket.recv()).await;
    let hello = match hello {
        Ok(Some(Ok(WsMessage::Text(text)))) => serde_json::from_str::<DeviceFrame>(&text).ok(),
        _ => None,
    };
    let Some(DeviceFrame::Hello {
        host_id,
        host_token,
        protocol,
    }) = hello
    else {
        let _ = socket
            .send(WsMessage::Close(Some(axum::extract::ws::CloseFrame {
                code: 1008,
                reason: "the first link frame must be hello".into(),
            })))
            .await;
        return;
    };
    if protocol != LINK_PROTOCOL {
        let _ = socket
            .send(WsMessage::Close(Some(axum::extract::ws::CloseFrame {
                code: 1008,
                reason: format!("unsupported link protocol {protocol}").into(),
            })))
            .await;
        return;
    }
    let host = match handlers_runtime_hosts::authenticate_host_token(&state, &host_id, &host_token)
        .await
    {
        Ok(host) => host,
        Err(error) => {
            tracing::warn!(%host_id, error = %error.0, "rejected host link");
            let _ = socket
                .send(WsMessage::Close(Some(axum::extract::ws::CloseFrame {
                    code: 4401,
                    reason: "invalid runtime host token".into(),
                })))
                .await;
            return;
        }
    };

    let (outbound_tx, mut outbound_rx) = mpsc::channel(OUTBOUND_CAPACITY);
    let link = Arc::new(HostLink {
        host_id: host.id.clone(),
        host_name: host.name.clone(),
        outbound: outbound_tx,
        pending: StdMutex::new(HashMap::new()),
        terminals: StdMutex::new(HashMap::new()),
        closed: Notify::new(),
    });
    if let Some(previous) = state.host_links.register(Arc::clone(&link)) {
        tracing::info!(host_id = %host.id, "replacing an existing host link");
        previous.close();
    }
    // Registration and revocation race independently of the initial hello.
    // Publish before rechecking so revocation either removes this link or the
    // second authorization check rejects a token fenced before publication.
    if handlers_runtime_hosts::authenticate_host_token(&state, &host_id, &host_token)
        .await
        .is_err()
    {
        state.host_links.unregister(&link);
        link.close();
        let _ = socket
            .send(WsMessage::Close(Some(axum::extract::ws::CloseFrame {
                code: 4401,
                reason: "runtime host authorization was revoked".into(),
            })))
            .await;
        return;
    }
    let welcome = serde_json::to_string(&ControllerFrame::Welcome {
        host_name: host.name.clone(),
    })
    .expect("welcome frame serialises");
    if socket.send(WsMessage::Text(welcome.into())).await.is_err() {
        state.host_links.unregister(&link);
        return;
    }
    if let Err(error) = handlers_runtime_hosts::mark_host_seen(&state, &host.id).await {
        tracing::warn!(host_id = %host.id, error = %error.0, "host link presence was not recorded");
    }
    tracing::info!(host_id = %host.id, host = %host.name, "host link established");

    let (mut sink, mut source) = socket.split();
    loop {
        tokio::select! {
            _ = link.closed.notified() => {
                let _ = sink.send(WsMessage::Close(None)).await;
                break;
            }
            frame = source.next() => {
                match frame {
                    Some(Ok(WsMessage::Text(text))) => handle_device_frame(&link, &text),
                    Some(Ok(WsMessage::Ping(bytes))) => {
                        if sink.send(WsMessage::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                }
            }
            outgoing = outbound_rx.recv() => {
                let Some(text) = outgoing else { break };
                if sink.send(WsMessage::Text(text.into())).await.is_err() {
                    break;
                }
            }
        }
    }
    state.host_links.unregister(&link);
    link.close();
    tracing::info!(host_id = %host.id, host = %host.name, "host link closed");
}

fn handle_device_frame(link: &Arc<HostLink>, text: &str) {
    let frame = match serde_json::from_str::<DeviceFrame>(text) {
        Ok(frame) => frame,
        Err(error) => {
            tracing::warn!(host_id = %link.host_id, %error, "ignored undecodable device frame");
            return;
        }
    };
    match frame {
        DeviceFrame::Hello { .. } => {}
        DeviceFrame::Result { id, ok, error } => {
            let outcome = match error {
                Some(error) => Err(error.into()),
                None => Ok(ok.unwrap_or(Value::Null)),
            };
            link.settle(&id, outcome);
        }
        DeviceFrame::TerminalOutput { terminal_id, data } => {
            let sender = link
                .terminals
                .lock()
                .expect("terminal outputs lock")
                .get(&terminal_id)
                .cloned();
            let Some(sender) = sender else { return };
            match choruz_host_runtime::link::decode_bytes(&data) {
                Ok(bytes) => {
                    if let Err(mpsc::error::TrySendError::Full(_)) = sender.sender.try_send(bytes) {
                        tracing::warn!(host_id = %link.host_id, terminal_id, "terminal output lost synchronization; reconnecting for a snapshot");
                        sender.resync.store(true, Ordering::Release);
                        link.unsubscribe_terminal(&terminal_id);
                    }
                }
                Err(error) => {
                    tracing::warn!(host_id = %link.host_id, terminal_id, %error, "dropped terminal frame");
                }
            }
        }
        DeviceFrame::TerminalExit { terminal_id } => link.unsubscribe_terminal(&terminal_id),
        DeviceFrame::TerminalResync { terminal_id } => {
            if let Some(subscriber) = link
                .terminals
                .lock()
                .expect("terminal outputs lock")
                .remove(&terminal_id)
            {
                subscriber.resync.store(true, Ordering::Release);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[tokio::test]
    async fn lagging_terminal_attachment_recovers_the_same_device_screen() {
        use super::*;
        use crate::host_runtime::RuntimeHost;
        use choruz_host_runtime::link::{DeviceIdentity, encode_bytes, run_device_link};
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let cli = directory.path().join("cli");
        std::fs::write(&cli, "#!/bin/sh\nprintf '\\033[2J\\033[HDEVICE READY\\r\\n'\nwhile IFS= read -r line; do printf '%s\\r\\n' \"$line\"; done\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let pool = choruz_host_runtime::new_terminal_pool();
        let hub = HostLinkHub::default();
        let (link, mut calls) = hub.decision_fixture("screen-device");
        let (to_device, incoming) = mpsc::channel(64);
        let (outgoing, mut from_device) = mpsc::channel(64);
        let device = tokio::spawn(run_device_link(
            outgoing,
            incoming,
            DeviceIdentity {
                host_id: "screen-device".into(),
                host_token: "fixture".into(),
            },
            pool.clone(),
        ));
        let hello: DeviceFrame = serde_json::from_str(&from_device.recv().await.unwrap()).unwrap();
        assert!(matches!(hello, DeviceFrame::Hello { .. }));
        to_device
            .send(
                serde_json::to_string(&ControllerFrame::Welcome {
                    host_name: "Device B".into(),
                })
                .unwrap(),
            )
            .await
            .unwrap();
        let routing_link = link.clone();
        let router = tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(call)=calls.recv()=>{ if to_device.send(call).await.is_err(){break;} }
                    Some(frame)=from_device.recv()=>handle_device_frame(&routing_link,&frame),
                    else=>break,
                }
            }
        });
        let host = RuntimeHost::Linked(link.clone());
        host.ensure_terminal(choruz_host_runtime::TerminalSpec {
            authentication: false,
            terminal_id: "screen".into(),
            driver_type: "claude_terminal".into(),
            binary_path: Some(cli.to_string_lossy().into_owned()),
            workspace_path: directory.path().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
            resume_session_id: None,
            codex_home: None,
            model: None,
            harness_account: serde_json::json!({}),
        })
        .await
        .unwrap();
        let original = pool.lock().unwrap().get("screen").unwrap().clone();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if original
                    .subscribe_with_replay()
                    .0
                    .concat()
                    .windows(12)
                    .any(|part| part == b"DEVICE READY")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("real device child produces its screen");
        let mut attachment = host.attach_terminal("screen").await.unwrap();
        // Saturate only this controller consumer. The actual device process,
        // link dispatcher and snapshot remain real; no successful reply is mocked.
        for _ in 0..=TERMINAL_OUTPUT_CAPACITY {
            let frame = DeviceFrame::TerminalOutput {
                terminal_id: "screen".into(),
                data: encode_bytes(b"incremental\r\n"),
            };
            handle_device_frame(&link, &serde_json::to_string(&frame).unwrap());
        }
        assert!(attachment.resync.load(Ordering::Acquire));
        while attachment.output.recv().await.is_some() {}
        let restored = host.attach_terminal("screen").await.unwrap();
        assert!(!restored.resync.load(Ordering::Acquire));
        assert!(String::from_utf8_lossy(&restored.replay.concat()).contains("DEVICE READY"));
        assert!(
            Arc::ptr_eq(&original, pool.lock().unwrap().get("screen").unwrap()),
            "recovery must not restart or duplicate native work"
        );
        assert!(original.is_child_alive());
        let _ = host.close_terminal("screen").await;
        router.abort();
        let _ = router.await;
        device.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn browser_call_waits_for_the_device_budget_and_cleanup() {
        use super::*;
        let hub = HostLinkHub::default();
        let (link, mut requests) = hub.decision_fixture("browser-budget");
        let request = serde_json::from_value(serde_json::json!({
            "browser":"fixture", "url":"https://example.org/", "model":"fixture", "minimum_confidence":0.9,
            "workflow":{"name":"Save","allowed_urls":["https://example.org/"],"steps":[{"goal":"Save","operation":"click","labels":["button \"Save\""],"value_key":null}]},
            "values":{}, "expected_text":["Saved"]
        })).unwrap();
        let call = link.call(LinkRequest::Host {
            request: choruz_host_runtime::HostRequest::BrowserWorkflow {
                id: "budget".into(),
                expires_at: 120,
                request,
                assistant: None,
            },
        });
        let device = async {
            let frame = requests.recv().await.unwrap();
            let ControllerFrame::Call { id, .. } = serde_json::from_str(&frame).unwrap() else {
                panic!("call");
            };
            tokio::time::sleep(Duration::from_secs(130)).await;
            link.settle(&id, Ok(serde_json::json!({"cleaned_up":true})));
        };
        let (result, ()) = tokio::join!(call, device);
        assert_eq!(result.unwrap()["cleaned_up"], true);
    }

    /// The route table names the path as a literal so the OpenAPI coverage
    /// test can read it; the connector dials the crate constant.
    #[test]
    fn the_link_route_is_the_path_devices_dial() {
        let routes = include_str!("plugins/remote_control.rs");
        assert!(routes.contains(&format!("\"{}\"", choruz_host_runtime::link::LINK_PATH)));
    }
}
