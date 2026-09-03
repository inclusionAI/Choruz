use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex},
    time::Instant,
};

use choruz_agent_runtime::RuntimeStore;
use choruz_application::ChatApp;
use choruz_session::PgSessionStore;

use crate::attachments::AttachmentStore;
use crate::local_auth::LocalAuthConfig;

pub(crate) struct PtySession {
    pub(crate) writer: Arc<StdMutex<Box<dyn std::io::Write + Send>>>,
    pub(crate) output_tx: tokio::sync::broadcast::Sender<Vec<u8>>,
    pub(crate) startup_replay: Arc<StdMutex<Option<Vec<Vec<u8>>>>>,
    pub(crate) child: StdMutex<Box<dyn portable_pty::Child + Send + Sync>>,
    pub(crate) master: Arc<StdMutex<Box<dyn portable_pty::MasterPty + Send>>>,
    pub(crate) last_accessed: StdMutex<Instant>,
    /// Cross-platform process container that wraps the child's entire process
    /// tree.  On eviction / drop the container kills all descendants — not
    /// just the direct child PID.
    pub(crate) _container: crate::pty_manager::ProcessContainer,
}

impl PtySession {
    pub(crate) fn touch(&self) {
        *self.last_accessed.lock().expect("last_accessed lock") = Instant::now();
    }

    /// Check if the child CLI process is still alive.
    pub(crate) fn is_child_alive(&self) -> bool {
        let mut child = self.child.lock().expect("child lock");
        // try_wait returns Ok(Some(status)) if exited, Ok(None) if still running
        match child.try_wait() {
            Ok(Some(_status)) => false, // exited
            Ok(None) => true,           // still running
            Err(_) => false,            // error checking = assume dead
        }
    }
}

pub(crate) type PtyPool = Arc<StdMutex<HashMap<String, Arc<PtySession>>>>;

/// Remove PTY sessions whose child process has exited.
/// When the last `Arc<PtySession>` is dropped the embedded `ProcessContainer`
/// kills the *entire* child process tree (not just the direct child PID).
///
/// Note: idle-based eviction was intentionally removed — PTY sessions should
/// persist until the process naturally exits or is explicitly killed.
pub(crate) fn evict_stale_pty_sessions(pool: &PtyPool) {
    let mut sessions = pool.lock().expect("pty pool lock");
    let before = sessions.len();
    sessions.retain(|id, session| {
        if !session.is_child_alive() {
            tracing::info!(binding_id = %id, "evicting PTY session with dead child process");
            false
        } else {
            true
        }
    });
    let evicted = before - sessions.len();
    if evicted > 0 {
        tracing::info!(
            evicted,
            remaining = sessions.len(),
            "PTY session eviction complete"
        );
    }
}

pub(crate) struct EnsureResult {
    pub(crate) session: Arc<PtySession>,
    pub(crate) newly_created: bool,
}

#[derive(Clone)]
pub struct ApiState {
    pub app: ChatApp,
    /// Stateless DB service — Phase 0 scaffolding for the stateless migration.
    /// Will progressively replace in-memory ChatApp methods.
    pub db: choruz_application::DbService,
    pub runtime: RuntimeStore,
    pub session: PgSessionStore,
    pub event_store: choruz_store::EventStore,
    pub attachments: AttachmentStore,
    pub auth: LocalAuthConfig,
    pub(crate) sync_wakeups: crate::sync_wakeup::SyncWakeupHub,
    pub(crate) remote_control_bridges: crate::remote_control_bridge::RemoteControlBridgeHub,
    pub(crate) pty_pool: PtyPool,
}
