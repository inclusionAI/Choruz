//! The controller side of the host link: every connected device holds one
//! WebSocket to `GET /v1/ws/runtime-hosts/link`, greets with its host token,
//! and from then on answers the controller's calls and streams terminal
//! output. `HostLinkHub` is the registry a handler asks for a device.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex},
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
use tokio::sync::{mpsc, oneshot};

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
    pub(crate) fn get(&self, host_id: &str) -> Option<Arc<HostLink>> {
        self.links
            .lock()
            .expect("host links lock")
            .get(host_id)
            .cloned()
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
    terminals: StdMutex<HashMap<String, mpsc::Sender<Vec<u8>>>>,
}

impl HostLink {
    /// Send one request to the device and wait for its reply.
    pub(crate) async fn call(&self, request: LinkRequest) -> Result<Value, AppError> {
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
        match tokio::time::timeout(CALL_TIMEOUT, reply_rx).await {
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
    pub(crate) fn subscribe_terminal(&self, terminal_id: &str) -> mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = mpsc::channel(TERMINAL_OUTPUT_CAPACITY);
        self.terminals
            .lock()
            .expect("terminal outputs lock")
            .insert(terminal_id.to_owned(), tx);
        rx
    }

    pub(crate) fn unsubscribe_terminal(&self, terminal_id: &str) {
        self.terminals
            .lock()
            .expect("terminal outputs lock")
            .remove(terminal_id);
    }

    fn settle(&self, id: &str, outcome: Result<Value, AppError>) {
        if let Some(reply) = self.pending.lock().expect("pending calls lock").remove(id) {
            let _ = reply.send(outcome);
        }
    }

    fn close(&self) {
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
    });
    if let Some(previous) = state.host_links.register(Arc::clone(&link)) {
        tracing::info!(host_id = %host.id, "replacing an existing host link");
        previous.close();
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
                    // A consumer that cannot keep up drops frames, as a lagging
                    // local subscriber does; the terminal repaints on demand.
                    if let Err(mpsc::error::TrySendError::Full(_)) = sender.try_send(bytes) {
                        tracing::warn!(host_id = %link.host_id, terminal_id, "terminal output consumer lagged, skipping frame");
                    }
                }
                Err(error) => {
                    tracing::warn!(host_id = %link.host_id, terminal_id, %error, "dropped terminal frame");
                }
            }
        }
        DeviceFrame::TerminalExit { terminal_id } => link.unsubscribe_terminal(&terminal_id),
    }
}

#[cfg(test)]
mod tests {
    /// The route table names the path as a literal so the OpenAPI coverage
    /// test can read it; the connector dials the crate constant.
    #[test]
    fn the_link_route_is_the_path_devices_dial() {
        let routes = include_str!("plugins/remote_control.rs");
        assert!(routes.contains(&format!("\"{}\"", choruz_host_runtime::link::LINK_PATH)));
    }
}
