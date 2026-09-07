//! The host link: one WebSocket a device keeps open to the controller, over
//! which the controller sends every request it has for that device and the
//! device streams terminal output back.
//!
//! A device dials the controller (it may sit behind the encrypted relay with
//! no inbound port), greets with [`DeviceFrame::Hello`] and then answers
//! [`ControllerFrame::Call`]s. All frames are JSON text; terminal bytes are
//! base64 inside them. [`run_device_link`] is the device side and is
//! transport-agnostic: it speaks through a pair of channels the caller wires
//! to a direct WebSocket or to a relay stream.

use std::{collections::HashMap, sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use choruz_common::AppError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{HostRequest, TerminalPool, TerminalSpec};

pub const LINK_PROTOCOL: u32 = 1;
/// The WebSocket path a device opens on the controller.
pub const LINK_PATH: &str = "/v1/ws/runtime-hosts/link";

/// Frames a device sends to the controller.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeviceFrame {
    Hello {
        host_id: String,
        host_token: String,
        protocol: u32,
    },
    /// The reply to a `Call`: `error` when the request failed, else `ok`
    /// (absent when the reply is JSON null).
    Result {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ok: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<LinkError>,
    },
    TerminalOutput {
        terminal_id: String,
        /// Base64 of the raw PTY bytes.
        data: String,
    },
    /// The terminal's child exited or the terminal was closed; no more
    /// output follows for this attachment.
    TerminalExit { terminal_id: String },
}

/// Frames the controller sends to a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControllerFrame {
    Welcome {
        host_name: String,
    },
    Call {
        id: String,
        request: Box<LinkRequest>,
    },
}

/// One request the controller can make of a device: a [`HostRequest`] or a
/// terminal operation. Terminal output is not a reply; it streams as
/// [`DeviceFrame::TerminalOutput`] after `TerminalAttach` succeeds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum LinkRequest {
    Session {
        request: crate::session::SessionRequest,
    },
    Host {
        request: HostRequest,
    },
    TerminalEnsure {
        spec: TerminalSpec,
    },
    TerminalAttach {
        terminal_id: String,
    },
    TerminalDetach {
        terminal_id: String,
    },
    TerminalWrite {
        terminal_id: String,
        data: String,
    },
    TerminalResize {
        terminal_id: String,
        cols: u16,
        rows: u16,
    },
    TerminalClose {
        terminal_id: String,
    },
    TerminalAlive {
        terminal_id: String,
    },
}

/// An [`AppError`] on the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkError {
    pub kind: String,
    pub message: String,
}

impl From<AppError> for LinkError {
    fn from(error: AppError) -> Self {
        let (kind, message) = match error {
            AppError::Unauthorized(message) => ("unauthorized", message),
            AppError::NotFound(message) => ("not_found", message),
            AppError::Conflict(message) => ("conflict", message),
            AppError::Validation(message) => ("validation", message),
            AppError::Forbidden(message) => ("forbidden", message),
            AppError::RateLimited { retry_after_ms } => {
                ("rate_limited", format!("retry after {retry_after_ms} ms"))
            }
            AppError::Internal(message) => ("internal", message),
        };
        Self {
            kind: kind.into(),
            message,
        }
    }
}

impl From<LinkError> for AppError {
    fn from(error: LinkError) -> Self {
        match error.kind.as_str() {
            "unauthorized" => AppError::Unauthorized(error.message),
            "not_found" => AppError::NotFound(error.message),
            "conflict" => AppError::Conflict(error.message),
            "validation" => AppError::Validation(error.message),
            "forbidden" => AppError::Forbidden(error.message),
            _ => AppError::Internal(error.message),
        }
    }
}

pub fn encode_bytes(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn decode_bytes(text: &str) -> Result<Vec<u8>, AppError> {
    STANDARD
        .decode(text)
        .map_err(|error| AppError::Validation(format!("invalid base64 terminal data: {error}")))
}

#[derive(Debug, Clone)]
pub struct DeviceIdentity {
    pub host_id: String,
    pub host_token: String,
}

/// Interval at which an attached terminal's child is checked for exit.
const LIVENESS_INTERVAL: Duration = Duration::from_millis(750);

/// Serve the controller over one link until either channel closes. Returns
/// the reason the link ended. Every terminal attachment the controller
/// opened is dropped with the link; the terminals themselves stay in `pool`
/// so a reconnect can re-attach.
pub async fn run_device_link(
    outbound: mpsc::Sender<String>,
    mut inbound: mpsc::Receiver<String>,
    identity: DeviceIdentity,
    pool: TerminalPool,
) -> Result<(), String> {
    send_frame(
        &outbound,
        &DeviceFrame::Hello {
            host_id: identity.host_id.clone(),
            host_token: identity.host_token.clone(),
            protocol: LINK_PROTOCOL,
        },
    )
    .await?;
    let welcome = inbound
        .recv()
        .await
        .ok_or("the controller closed the link before welcoming the device")?;
    match serde_json::from_str::<ControllerFrame>(&welcome) {
        Ok(ControllerFrame::Welcome { host_name }) => {
            tracing::info!(host = %host_name, "host link established");
        }
        Ok(other) => return Err(format!("unexpected first link frame: {other:?}")),
        Err(error) => return Err(format!("undecodable first link frame: {error}")),
    }

    let attachments: Attachments = Arc::new(std::sync::Mutex::new(HashMap::new()));
    let result = loop {
        let Some(text) = inbound.recv().await else {
            break Ok(());
        };
        let frame = match serde_json::from_str::<ControllerFrame>(&text) {
            Ok(frame) => frame,
            Err(error) => {
                tracing::warn!(%error, "ignored undecodable link frame");
                continue;
            }
        };
        match frame {
            ControllerFrame::Welcome { .. } => {}
            ControllerFrame::Call { id, request } => {
                let outbound = outbound.clone();
                let pool = pool.clone();
                let attachments = Arc::clone(&attachments);
                tokio::spawn(async move {
                    let outcome = handle_call(*request, &pool, &outbound, &attachments).await;
                    let frame = match outcome {
                        Ok(ok) => DeviceFrame::Result {
                            id,
                            ok: Some(ok),
                            error: None,
                        },
                        Err(error) => DeviceFrame::Result {
                            id,
                            ok: None,
                            error: Some(error.into()),
                        },
                    };
                    if let Err(error) = send_frame(&outbound, &frame).await {
                        tracing::warn!(%error, "link reply was not delivered");
                    }
                });
            }
        }
    };
    for (_, forwarder) in attachments.lock().expect("attachments lock").drain() {
        forwarder.abort();
    }
    result
}

type Attachments = Arc<std::sync::Mutex<HashMap<String, JoinHandle<()>>>>;

async fn send_frame(outbound: &mpsc::Sender<String>, frame: &DeviceFrame) -> Result<(), String> {
    let text = serde_json::to_string(frame).map_err(|error| error.to_string())?;
    outbound
        .send(text)
        .await
        .map_err(|_| "the link transport closed".to_owned())
}

async fn handle_call(
    request: LinkRequest,
    pool: &TerminalPool,
    outbound: &mpsc::Sender<String>,
    attachments: &Attachments,
) -> Result<Value, AppError> {
    match request {
        LinkRequest::Session { request } => crate::session::execute(pool.clone(), request)
            .await
            .map(|state| json!(state)),
        LinkRequest::Host { request } => crate::execute(request).await,
        LinkRequest::TerminalEnsure { spec } => {
            let pool = pool.clone();
            let outcome = tokio::task::spawn_blocking(move || crate::ensure_terminal(&pool, &spec))
                .await
                .map_err(|error| {
                    AppError::Internal(format!("terminal spawn panicked: {error}"))
                })??;
            Ok(json!({ "newly_created": outcome.newly_created }))
        }
        LinkRequest::TerminalAttach { terminal_id } => {
            let session = pooled(pool, &terminal_id)?;
            stop_forwarder(attachments, &terminal_id);
            let (replay, mut output) = session.subscribe_with_replay();
            let replay = replay
                .iter()
                .map(|frame| encode_bytes(frame))
                .collect::<Vec<_>>();
            let outbound = outbound.clone();
            let id = terminal_id.clone();
            let forwarder = tokio::spawn(async move {
                loop {
                    tokio::select! {
                        frame = output.recv() => match frame {
                            Ok(data) => {
                                let frame = DeviceFrame::TerminalOutput {
                                    terminal_id: id.clone(),
                                    data: encode_bytes(&data),
                                };
                                if send_frame(&outbound, &frame).await.is_err() {
                                    return;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                tracing::warn!(skipped, terminal_id = %id, "terminal output lagged over the link");
                            }
                            Err(_) => break,
                        },
                        _ = tokio::time::sleep(LIVENESS_INTERVAL) => {
                            if !session.is_child_alive() {
                                break;
                            }
                        }
                    }
                }
                let _ = send_frame(
                    &outbound,
                    &DeviceFrame::TerminalExit {
                        terminal_id: id.clone(),
                    },
                )
                .await;
            });
            attachments
                .lock()
                .expect("attachments lock")
                .insert(terminal_id, forwarder);
            Ok(json!({ "replay": replay }))
        }
        LinkRequest::TerminalDetach { terminal_id } => {
            stop_forwarder(attachments, &terminal_id);
            Ok(Value::Null)
        }
        LinkRequest::TerminalWrite { terminal_id, data } => {
            let session = pooled(pool, &terminal_id)?;
            session.touch();
            session.write_all(&decode_bytes(&data)?)?;
            Ok(Value::Null)
        }
        LinkRequest::TerminalResize {
            terminal_id,
            cols,
            rows,
        } => {
            pooled(pool, &terminal_id)?.resize(cols, rows)?;
            Ok(Value::Null)
        }
        LinkRequest::TerminalClose { terminal_id } => {
            stop_forwarder(attachments, &terminal_id);
            crate::terminal::close_terminal(pool, &terminal_id)?;
            Ok(Value::Null)
        }
        LinkRequest::TerminalAlive { terminal_id } => {
            Ok(json!(crate::live_terminal_exists(pool, &terminal_id)))
        }
    }
}

fn pooled(pool: &TerminalPool, terminal_id: &str) -> Result<Arc<crate::TerminalSession>, AppError> {
    pool.lock()
        .expect("terminal pool lock")
        .get(terminal_id)
        .cloned()
        .ok_or_else(|| AppError::Validation("terminal not running".into()))
}

fn stop_forwarder(attachments: &Attachments, terminal_id: &str) {
    if let Some(forwarder) = attachments
        .lock()
        .expect("attachments lock")
        .remove(terminal_id)
    {
        forwarder.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn welcomed_link() -> (
        mpsc::Sender<String>,
        mpsc::Receiver<String>,
        JoinHandle<Result<(), String>>,
    ) {
        let (device_tx, mut controller_rx) = mpsc::channel(64);
        let (controller_tx, device_rx) = mpsc::channel(64);
        let link = tokio::spawn(run_device_link(
            device_tx,
            device_rx,
            DeviceIdentity {
                host_id: "host-1".into(),
                host_token: "token-1".into(),
            },
            crate::new_terminal_pool(),
        ));
        let hello: DeviceFrame =
            serde_json::from_str(&controller_rx.recv().await.expect("hello")).unwrap();
        assert!(matches!(
            hello,
            DeviceFrame::Hello { ref host_id, ref host_token, protocol }
                if host_id == "host-1" && host_token == "token-1" && protocol == LINK_PROTOCOL
        ));
        controller_tx
            .send(
                serde_json::to_string(&ControllerFrame::Welcome {
                    host_name: "Build Server".into(),
                })
                .unwrap(),
            )
            .await
            .unwrap();
        (controller_tx, controller_rx, link)
    }

    async fn call(
        controller_tx: &mpsc::Sender<String>,
        controller_rx: &mut mpsc::Receiver<String>,
        id: &str,
        request: LinkRequest,
    ) -> (DeviceFrame, Vec<DeviceFrame>) {
        controller_tx
            .send(
                serde_json::to_string(&ControllerFrame::Call {
                    id: id.into(),
                    request: Box::new(request),
                })
                .unwrap(),
            )
            .await
            .unwrap();
        let mut events = Vec::new();
        loop {
            let frame: DeviceFrame =
                serde_json::from_str(&controller_rx.recv().await.expect("reply")).unwrap();
            if matches!(&frame, DeviceFrame::Result { id: reply_id, .. } if reply_id == id) {
                return (frame, events);
            }
            // Terminal events can precede a call's reply on the same link.
            events.push(frame);
        }
    }

    #[tokio::test]
    async fn call_preserves_terminal_events_queued_before_its_reply() {
        let (controller_tx, _device_rx) = mpsc::channel(1);
        let (device_tx, mut controller_rx) = mpsc::channel(3);
        for frame in [
            DeviceFrame::TerminalOutput {
                terminal_id: "binding-link".into(),
                data: encode_bytes(b"READY"),
            },
            DeviceFrame::TerminalExit {
                terminal_id: "binding-link".into(),
            },
            DeviceFrame::Result {
                id: "alive".into(),
                ok: Some(json!(false)),
                error: None,
            },
        ] {
            device_tx
                .send(serde_json::to_string(&frame).unwrap())
                .await
                .unwrap();
        }
        let (reply, events) = call(
            &controller_tx,
            &mut controller_rx,
            "alive",
            LinkRequest::TerminalAlive {
                terminal_id: "binding-link".into(),
            },
        )
        .await;
        assert!(
            matches!(reply, DeviceFrame::Result { ok: Some(value), error: None, .. } if value == json!(false))
        );
        assert!(matches!(events.as_slice(), [
            DeviceFrame::TerminalOutput { terminal_id, data },
            DeviceFrame::TerminalExit { terminal_id: exit_id },
        ] if terminal_id == "binding-link" && exit_id == terminal_id && decode_bytes(data).unwrap() == b"READY"));
    }

    #[tokio::test]
    async fn device_link_greets_answers_host_requests_and_reports_errors() {
        let (controller_tx, mut controller_rx, link) = welcomed_link().await;

        let (reply, _) = call(
            &controller_tx,
            &mut controller_rx,
            "home",
            LinkRequest::Host {
                request: HostRequest::FilesystemHome,
            },
        )
        .await;
        let DeviceFrame::Result { ok: Some(ok), .. } = reply else {
            panic!("filesystem home must succeed: {reply:?}");
        };
        assert!(ok["home"].is_string());

        let (reply, _) = call(
            &controller_tx,
            &mut controller_rx,
            "missing",
            LinkRequest::TerminalWrite {
                terminal_id: "nope".into(),
                data: encode_bytes(b"hi"),
            },
        )
        .await;
        let DeviceFrame::Result {
            error: Some(error), ..
        } = reply
        else {
            panic!("writing to an unknown terminal must fail: {reply:?}");
        };
        assert_eq!(error.kind, "validation");
        assert_eq!(
            AppError::from(error).to_string(),
            "validation failed: terminal not running"
        );

        drop(controller_tx);
        assert_eq!(link.await.unwrap(), Ok(()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn device_link_streams_an_attached_terminal_and_its_exit() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let cli = temp.path().join("fake-cli.sh");
        std::fs::write(
            &cli,
            "#!/bin/sh\nprintf 'READY\\r\\n'\nIFS= read -r line\nprintf 'ECHO:%s\\r\\n' \"$line\"\nexit 0\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&cli).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&cli, permissions).unwrap();

        let (controller_tx, mut controller_rx, link) = welcomed_link().await;
        let spec = TerminalSpec {
            terminal_id: "binding-link".into(),
            driver_type: "claude_terminal".into(),
            binary_path: Some(cli.to_string_lossy().into_owned()),
            workspace_path: temp.path().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
            resume_session_id: None,
            codex_home: None,
            model: None,
            harness_account: json!({}),
        };
        let (reply, _) = call(
            &controller_tx,
            &mut controller_rx,
            "ensure",
            LinkRequest::TerminalEnsure { spec },
        )
        .await;
        assert!(
            matches!(&reply, DeviceFrame::Result { ok: Some(ok), .. } if ok["newly_created"] == true),
            "{reply:?}"
        );
        let (reply, mut events) = call(
            &controller_tx,
            &mut controller_rx,
            "attach",
            LinkRequest::TerminalAttach {
                terminal_id: "binding-link".into(),
            },
        )
        .await;
        let DeviceFrame::Result { ok: Some(ok), .. } = reply else {
            panic!("attach must succeed: {reply:?}");
        };
        let mut transcript = ok["replay"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|frame| decode_bytes(frame.as_str().unwrap()).unwrap())
            .collect::<Vec<u8>>();

        let (reply, write_events) = call(
            &controller_tx,
            &mut controller_rx,
            "write",
            LinkRequest::TerminalWrite {
                terminal_id: "binding-link".into(),
                data: encode_bytes(b"over the link\n"),
            },
        )
        .await;
        assert!(matches!(reply, DeviceFrame::Result { error: None, .. }));
        events.extend(write_events);
        let mut events = events.into_iter();

        let mut exited = false;
        tokio::time::timeout(Duration::from_secs(5), async {
            while !exited {
                let frame = match events.next() {
                    Some(frame) => frame,
                    None => {
                        serde_json::from_str(&controller_rx.recv().await.expect("frame")).unwrap()
                    }
                };
                match frame {
                    DeviceFrame::TerminalOutput { terminal_id, data } => {
                        assert_eq!(terminal_id, "binding-link");
                        transcript.extend(decode_bytes(&data).unwrap());
                    }
                    DeviceFrame::TerminalExit { terminal_id } => {
                        assert_eq!(terminal_id, "binding-link");
                        exited = true;
                    }
                    other => panic!("unexpected frame {other:?}"),
                }
            }
        })
        .await
        .expect("the fake CLI exits after echoing");
        let transcript = String::from_utf8_lossy(&transcript);
        assert!(transcript.contains("READY"), "{transcript}");
        assert!(transcript.contains("ECHO:over the link"), "{transcript}");

        drop(controller_tx);
        assert_eq!(link.await.unwrap(), Ok(()));
    }
}
