use choruz_agent_runtime::RuntimeStore;
use choruz_application::ChatApp;
use choruz_session::PgSessionStore;

use crate::attachments::AttachmentStore;
use crate::local_auth::LocalAuthConfig;

#[derive(Clone)]
pub struct ApiState {
    pub(crate) experience_worker: Option<std::sync::Arc<crate::experience_worker::WorkerGuard>>,
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
    /// The gateway's own device.
    pub(crate) local_host: crate::host_runtime::LocalHost,
    /// Every remote device that currently holds a host link.
    pub(crate) host_links: crate::host_link::HostLinkHub,
    pub(crate) online: crate::online_groups::OnlineHub,
}
