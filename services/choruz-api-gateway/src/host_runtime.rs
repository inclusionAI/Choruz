//! The device an agent binding runs on, behind one interface.
//!
//! Every handler that needs a terminal, a directory listing, a session
//! catalog or a Codex home asks a [`RuntimeHost`] and never looks at where
//! the device is. [`LocalHost`] serves the gateway's own device in-process
//! through `choruz-host-runtime`; [`RuntimeHost::Linked`] sends the same
//! requests over a device's host link.

use std::sync::Arc;

use choruz_agent_runtime::RuntimeBinding;
use choruz_common::AppError;
use choruz_host_runtime::{
    HostRequest, TerminalPool, TerminalSpec,
    link::{LinkRequest, decode_bytes, encode_bytes},
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::{ApiState, host_link::HostLink};

/// The bytes a terminal produces after a reader attaches: what it wrote
/// before anyone listened, then a live stream that ends when the child exits
/// or the terminal is closed.
pub(crate) struct TerminalAttachment {
    pub(crate) replay: Vec<Vec<u8>>,
    pub(crate) output: mpsc::Receiver<Vec<u8>>,
}

#[derive(Clone)]
pub(crate) enum RuntimeHost {
    Local(LocalHost),
    Linked(Arc<HostLink>),
}

#[derive(Clone)]
pub(crate) struct LocalHost {
    pub(crate) terminals: TerminalPool,
}

impl LocalHost {
    pub(crate) fn new() -> Self {
        Self {
            terminals: choruz_host_runtime::new_terminal_pool(),
        }
    }
}

impl RuntimeHost {
    /// The device that runs `binding`: its `config_json.runtime_host_id`
    /// when it has one, else the gateway's own device.
    pub(crate) fn for_binding(
        state: &ApiState,
        binding: &RuntimeBinding,
    ) -> Result<Self, AppError> {
        match binding
            .config_json
            .get("runtime_host_id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
        {
            Some(host_id) => Self::for_host(state, host_id),
            None => Ok(Self::local(state)),
        }
    }

    /// A connected runtime host by id.
    pub(crate) fn for_host(state: &ApiState, host_id: &str) -> Result<Self, AppError> {
        state
            .host_links
            .get(host_id)
            .map(Self::Linked)
            .ok_or_else(|| AppError::Conflict(format!("runtime host {host_id} is not connected")))
    }

    /// The gateway's own device.
    pub(crate) fn local(state: &ApiState) -> Self {
        Self::Local(state.local_host.clone())
    }

    /// Run one request/response operation on the device and decode the reply.
    pub(crate) async fn call<T: DeserializeOwned>(
        &self,
        request: HostRequest,
    ) -> Result<T, AppError> {
        let value = match self {
            Self::Local(_) => choruz_host_runtime::execute(request).await?,
            Self::Linked(link) => link.call(LinkRequest::Host { request }).await?,
        };
        serde_json::from_value(value)
            .map_err(|error| AppError::Internal(format!("decode device reply: {error}")))
    }

    /// Reuse or spawn the terminal; returns whether it was newly created.
    pub(crate) async fn ensure_terminal(&self, spec: TerminalSpec) -> Result<bool, AppError> {
        match self {
            Self::Local(local) => {
                let pool = local.terminals.clone();
                tokio::task::spawn_blocking(move || {
                    choruz_host_runtime::ensure_terminal(&pool, &spec)
                        .map(|outcome| outcome.newly_created)
                })
                .await
                .map_err(|error| AppError::Internal(format!("terminal spawn panicked: {error}")))?
            }
            Self::Linked(link) => {
                let reply = link.call(LinkRequest::TerminalEnsure { spec }).await?;
                Ok(reply["newly_created"].as_bool().unwrap_or(false))
            }
        }
    }

    pub(crate) async fn attach_terminal(
        &self,
        terminal_id: &str,
    ) -> Result<TerminalAttachment, AppError> {
        match self {
            Self::Local(local) => {
                let session = local_session(&local.terminals, terminal_id)?;
                let (replay, mut broadcast) = session.subscribe_with_replay();
                let (tx, rx) = mpsc::channel(256);
                let liveness = Arc::clone(&session);
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            frame = broadcast.recv() => match frame {
                                Ok(data) => {
                                    if tx.send(data).await.is_err() {
                                        break;
                                    }
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                    tracing::warn!(skipped, "PTY output consumer lagged, skipping frames");
                                }
                                Err(_) => break,
                            },
                            // The session keeps its sender alive after the
                            // reader thread exits, so a dead child is detected
                            // by polling rather than by the channel closing.
                            _ = tokio::time::sleep(std::time::Duration::from_millis(750)) => {
                                if !liveness.is_child_alive() {
                                    break;
                                }
                            }
                        }
                    }
                });
                Ok(TerminalAttachment { replay, output: rx })
            }
            Self::Linked(link) => {
                let output = link.subscribe_terminal(terminal_id);
                let reply = match link
                    .call(LinkRequest::TerminalAttach {
                        terminal_id: terminal_id.to_owned(),
                    })
                    .await
                {
                    Ok(reply) => reply,
                    Err(error) => {
                        link.unsubscribe_terminal(terminal_id);
                        return Err(error);
                    }
                };
                let replay = reply["replay"]
                    .as_array()
                    .map(|frames| {
                        frames
                            .iter()
                            .filter_map(Value::as_str)
                            .filter_map(|frame| decode_bytes(frame).ok())
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(TerminalAttachment { replay, output })
            }
        }
    }

    pub(crate) async fn write_terminal(
        &self,
        terminal_id: &str,
        data: &[u8],
    ) -> Result<(), AppError> {
        match self {
            Self::Local(local) => {
                let session = local_session(&local.terminals, terminal_id)?;
                session.touch();
                session.write_all(data)
            }
            Self::Linked(link) => link
                .call(LinkRequest::TerminalWrite {
                    terminal_id: terminal_id.to_owned(),
                    data: encode_bytes(data),
                })
                .await
                .map(|_| ()),
        }
    }

    pub(crate) async fn resize_terminal(
        &self,
        terminal_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), AppError> {
        match self {
            Self::Local(local) => local_session(&local.terminals, terminal_id)?.resize(cols, rows),
            Self::Linked(link) => link
                .call(LinkRequest::TerminalResize {
                    terminal_id: terminal_id.to_owned(),
                    cols,
                    rows,
                })
                .await
                .map(|_| ()),
        }
    }

    pub(crate) async fn terminal_is_live(&self, terminal_id: &str) -> bool {
        match self {
            Self::Local(local) => {
                choruz_host_runtime::live_terminal_exists(&local.terminals, terminal_id)
            }
            Self::Linked(link) => link
                .call(LinkRequest::TerminalAlive {
                    terminal_id: terminal_id.to_owned(),
                })
                .await
                .ok()
                .and_then(|reply| reply.as_bool())
                .unwrap_or(false),
        }
    }

    /// Kill the terminal's process tree so the next attach starts a fresh
    /// `--resume`.
    pub(crate) async fn close_terminal(&self, terminal_id: &str) -> Result<(), AppError> {
        match self {
            Self::Local(local) => {
                choruz_host_runtime::terminal::close_terminal(&local.terminals, terminal_id)
            }
            Self::Linked(link) => {
                link.unsubscribe_terminal(terminal_id);
                link.call(LinkRequest::TerminalClose {
                    terminal_id: terminal_id.to_owned(),
                })
                .await
                .map(|_| ())
            }
        }
    }
}

fn local_session(
    pool: &TerminalPool,
    terminal_id: &str,
) -> Result<Arc<choruz_host_runtime::TerminalSession>, AppError> {
    pool.lock()
        .expect("terminal pool lock")
        .get(terminal_id)
        .cloned()
        .ok_or_else(|| AppError::Validation("terminal not running".into()))
}
