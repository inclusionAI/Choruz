mod attachments;
mod auth;
pub mod config;
mod db_projection;
mod handlers_channel_tasks;
mod handlers_companies;
mod handlers_conversations;
mod handlers_cron;
mod handlers_events;
mod handlers_filesystem;
mod handlers_harness_logins;
mod handlers_messages;
mod handlers_principals;
mod handlers_remote_control;
mod handlers_runtime;
mod handlers_runtime_hosts;
mod handlers_runtime_status;
mod handlers_ssh;
mod handlers_sync_ws;
mod handlers_tasks;
mod handlers_terminals;
mod handlers_threads;
mod handlers_workspace_sessions;
pub mod ingress;
mod keepalive;
mod local_auth;
mod meta_handlers;
mod plugins;
pub(crate) mod pty_manager;
mod remote_control_bridge;
mod remote_control_executor;
mod remote_control_pairing_host;
mod state;
mod sync_wakeup;
mod webhook;

pub use config::Config;
pub use local_auth::{LocalAuthConfig, cookie_name as session_cookie_name};
pub use state::ApiState;

// Re-export items used by handler modules and tests
pub(crate) use auth::{
    ApiError, authenticated_principal, bearer_token_value, redact_sensitive_text, require_actor,
    require_human_operator, require_self,
};
pub(crate) use db_projection::{db_persist, persist_principal_to_db};
pub(crate) use state::{EnsureResult, PtyPool, PtySession, evict_stale_pty_sessions};
pub(crate) use webhook::{WebhookFlushResponse, flush_webhooks, flush_webhooks_all};

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::OnceLock;

    pub(crate) fn api_test_env_lock() -> &'static tokio::sync::Mutex<()> {
        static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
    }
}

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
};

use attachments::AttachmentStore;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{delete, get, patch, post, put},
};
use choruz_agent_runtime::RuntimeStore;
use choruz_application::ChatApp;
use choruz_session::PgSessionStore;
pub fn router(app: ChatApp) -> Router {
    let attachment_root = std::env::var("CHORUZ_ATTACHMENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".choruz-runtime/attachments"));
    let database_url = choruz_common::PgConfig::from_env().to_connect_string();
    router_with_runtime(
        app,
        attachment_root,
        LocalAuthConfig::from_env(),
        RuntimeStore::new(database_url.clone()),
        PgSessionStore::new(&database_url),
        choruz_store::EventStore::new(database_url),
    )
}

pub fn router_with_attachment_root(app: ChatApp, attachment_root: impl Into<PathBuf>) -> Router {
    let database_url = choruz_common::PgConfig::from_env().to_connect_string();
    router_with_runtime(
        app,
        attachment_root,
        LocalAuthConfig::from_env(),
        RuntimeStore::new(database_url.clone()),
        PgSessionStore::new(&database_url),
        choruz_store::EventStore::new(database_url),
    )
}

pub fn router_with_runtime(
    app: ChatApp,
    attachment_root: impl Into<PathBuf>,
    auth: LocalAuthConfig,
    runtime: RuntimeStore,
    session: PgSessionStore,
    event_store: choruz_store::EventStore,
) -> Router {
    // Create the PTY pool before the router so it can be shared with the
    // keepalive task.
    let pty_pool: PtyPool = Arc::new(StdMutex::new(HashMap::new()));

    // Spawn the agent keepalive background task (60s interval).
    keepalive::spawn_keepalive_task(event_store.clone(), pty_pool.clone());

    Router::new()
        .route("/healthz", get(meta_handlers::liveness))
        .route("/readyz", get(meta_handlers::readiness))
        .route("/metrics", get(meta_handlers::metrics))
        .route("/v1/status", get(meta_handlers::phase_status))
        .route(
            "/v1/auth/local/login",
            post(handlers_principals::local_login),
        )
        .route(
            "/v1/auth/local/bootstrap",
            get(handlers_principals::local_bootstrap),
        )
        .route(
            "/v1/auth/local/signup",
            post(handlers_principals::local_signup),
        )
        .route("/v1/me", get(handlers_principals::me))
        .route("/v1/bootstrap", get(meta_handlers::bootstrap))
        .route("/v1/sync", get(meta_handlers::sync_changes))
        .route("/v1/ws/sync", get(handlers_sync_ws::sync_socket))
        .route("/v1/console", get(meta_handlers::console_snapshot))
        .route(
            "/v1/runtime/bindings",
            get(handlers_runtime::list_runtime_bindings)
                .post(handlers_runtime::create_runtime_binding),
        )
        .route(
            "/v1/runtime/bindings/{binding_id}",
            get(handlers_runtime::get_runtime_binding),
        )
        .route(
            "/v1/runtime/bindings/{binding_id}/rebind",
            post(handlers_runtime::rebind_runtime_binding),
        )
        .route(
            "/v1/agents/{agent_id}/tasks",
            get(handlers_tasks::get_agent_tasks),
        )
        .route(
            "/v1/agents/{agent_id}/cron",
            get(handlers_cron::list_cron_jobs).post(handlers_cron::create_cron_job),
        )
        .route(
            "/v1/agents/{agent_id}/cron/{job_id}",
            patch(handlers_cron::update_cron_job).delete(handlers_cron::delete_cron_job),
        )
        .route(
            "/v1/companies/{company_id}/reset-sessions",
            post(handlers_runtime::reset_company_sessions),
        )
        .route(
            "/v1/runtime/policies/{conversation_id}",
            get(handlers_runtime::get_runtime_policy).put(handlers_runtime::upsert_runtime_policy),
        )
        .route(
            "/v1/conversations/{conversation_id}/runtime-status",
            get(handlers_runtime_status::get_conversation_runtime_status),
        )
        .route(
            "/v1/principals/{principal_id}/disable",
            post(handlers_principals::disable_principal),
        )
        .route(
            "/v1/principals/{principal_id}/workspace",
            patch(handlers_principals::migrate_principal_workspace),
        )
        .route("/v1/agents", post(handlers_principals::create_agent))
        .route(
            "/v1/agents/batch-disable",
            post(handlers_principals::batch_disable_agents),
        )
        .route(
            "/v1/agents/{agent_id}/rotate-secret",
            post(handlers_principals::rotate_agent_secret),
        )
        .route(
            "/v1/conversations",
            get(handlers_conversations::list_conversations),
        )
        .route(
            "/v1/conversations/direct",
            post(handlers_conversations::create_direct_conversation),
        )
        .route(
            "/v1/conversations/{conversation_id}/workspace",
            patch(handlers_conversations::migrate_conversation_workspace),
        )
        .route(
            "/v1/conversations/{conversation_id}/pin",
            put(handlers_conversations::pin_conversation)
                .delete(handlers_conversations::unpin_conversation),
        )
        .route(
            "/v1/conversations/{conversation_id}/archive",
            put(handlers_conversations::archive_conversation)
                .delete(handlers_conversations::unarchive_conversation),
        )
        .route(
            "/v1/conversations/{conversation_id}/hide",
            put(handlers_conversations::hide_agent_session)
                .delete(handlers_conversations::restore_hidden_agent_session),
        )
        .route(
            "/v1/conversations/{conversation_id}/messages",
            get(handlers_messages::list_messages),
        )
        .route(
            "/v1/conversations/{conversation_id}/message-page",
            get(handlers_messages::list_message_page),
        )
        .route(
            "/v1/conversations/{conversation_id}/messages/{message_id}",
            get(handlers_messages::get_message),
        )
        .route(
            "/v1/companies/{company_id}/harness-accounts/{account_id}/logins",
            post(handlers_harness_logins::start_harness_account_login),
        )
        .route(
            "/v1/companies/{company_id}/harness-accounts/{account_id}/logins/{login_id}",
            get(handlers_harness_logins::get_harness_account_login),
        )
        .route(
            "/v1/companies/{company_id}/harness-accounts/{account_id}/logins/{login_id}/callback",
            post(handlers_harness_logins::submit_harness_account_login_callback),
        )
        .route(
            "/v1/companies/{company_id}/harness-accounts/{account_id}/logins/{login_id}/cancel",
            post(handlers_harness_logins::cancel_harness_account_login),
        )
        .merge(plugins::router())
        .route(
            "/v1/conversations/{conversation_id}/view",
            post(handlers_messages::view_conversation),
        )
        // ── Threads ───────────────
        // Threaded replies are written via POST /v1/messages with
        // metadata {reply_to_id, thread: true}; these routes are the
        // read side only.
        .route(
            "/v1/conversations/{conversation_id}/threads/{thread_root_id}",
            get(handlers_threads::get_thread),
        )
        .route(
            "/v1/conversations/{conversation_id}/threads/{thread_root_id}/view",
            post(handlers_threads::view_thread),
        )
        .route("/v1/unreads", get(handlers_messages::get_unreads))
        .route("/v1/groups", post(handlers_conversations::create_group))
        .route(
            "/v1/groups/{conversation_id}",
            patch(handlers_conversations::update_group),
        )
        .route(
            "/v1/groups/{conversation_id}/members",
            post(handlers_conversations::add_group_members),
        )
        .route(
            "/v1/groups/{conversation_id}/members/{principal_id}",
            delete(handlers_conversations::remove_group_member),
        )
        .route("/v1/messages", post(handlers_messages::send_message))
        .route(
            "/v1/messages/search",
            get(handlers_messages::search_messages),
        )
        // Per-route body cap: attachment upload body is base64-encoded bytes
        // + JSON metadata, so a 25 MB binary inflates to ≈ 33.3 MB. 34 MB
        // accepts that worst case; everything else is still bounded by
        // axum's 2 MB default. `attachments::MAX_ATTACHMENT_BYTES`
        // re-enforces on the decoded payload as defence in depth.
        .route(
            "/v1/attachments",
            post(handlers_messages::upload_attachment)
                .layer(DefaultBodyLimit::max(34 * 1024 * 1024)),
        )
        .route(
            "/v1/attachments/{attachment_id}",
            get(handlers_messages::download_attachment)
                .delete(handlers_messages::delete_attachment),
        )
        .route(
            "/v1/principals/{principal_id}/events",
            get(handlers_events::list_events),
        )
        .route(
            "/v1/principals/{principal_id}/events/ack",
            post(handlers_events::ack_events),
        )
        .route(
            "/v1/principals/{principal_id}/event-webhook",
            post(handlers_events::set_event_webhook),
        )
        .route(
            "/v1/webhooks/flush",
            post(handlers_events::flush_webhook_deliveries),
        )
        // Old /v1/ws/events polling WS removed — use fanout WS (/ws/fanout on pipeline port) instead.
        .route(
            "/v1/ws/terminals/{binding_id}",
            get(handlers_terminals::websocket_terminal),
        )
        .route("/v1/telemetry", post(handlers_events::ingest_telemetry))
        .route(
            "/v1/terminals/{binding_id}/ensure",
            post(handlers_terminals::ensure_terminal),
        )
        .route(
            "/v1/terminals/{binding_id}/input",
            post(handlers_terminals::terminal_input),
        )
        // ── Company routes ──
        .route(
            "/v1/companies",
            get(handlers_companies::list_companies).post(handlers_companies::create_company),
        )
        .route(
            "/v1/companies/{company_id}",
            get(handlers_companies::get_company)
                .patch(handlers_companies::update_company)
                .delete(handlers_companies::delete_company),
        )
        .route(
            "/v1/companies/{company_id}/archive",
            post(handlers_companies::archive_company),
        )
        .route(
            "/v1/companies/{company_id}/unarchive",
            post(handlers_companies::unarchive_company),
        )
        .route(
            "/v1/companies/{company_id}/members",
            get(handlers_companies::list_company_members)
                .post(handlers_companies::add_company_member),
        )
        .route(
            "/v1/companies/{company_id}/members/{member_id}",
            delete(handlers_companies::remove_company_member),
        )
        .route("/v1/audit-logs", get(handlers_companies::list_audit_logs))
        .route(
            "/v1/export/conversations/{conversation_id}",
            get(handlers_companies::export_conversation),
        )
        .route(
            "/v1/filesystem/list",
            get(handlers_filesystem::filesystem_list),
        )
        .route(
            "/v1/filesystem/stat",
            get(handlers_filesystem::filesystem_stat),
        )
        .route(
            "/v1/filesystem/home",
            get(handlers_filesystem::filesystem_home),
        )
        .route(
            "/v1/filesystem/read",
            get(handlers_filesystem::filesystem_read),
        )
        .route(
            "/v1/filesystem/write",
            post(handlers_filesystem::filesystem_write),
        )
        // ── New message pipeline (Phase B) ──
        .route("/v2/ingest", post(ingress::ingest_message))
        // (observability routes are merged below, after .with_state())
        .with_state({
            let db = choruz_application::DbService::new(event_store.clone());
            let attachments = AttachmentStore::new(attachment_root.into(), event_store.clone());
            let sync_wakeups =
                sync_wakeup::SyncWakeupHub::spawn(event_store.database_url().to_owned());

            let (remote_control_bridges, bridge_refreshes) =
                remote_control_bridge::RemoteControlBridgeHub::new();
            let state = ApiState {
                app,
                db,
                runtime,
                session,
                event_store,
                attachments,
                auth,
                sync_wakeups,
                remote_control_bridges,
                pty_pool,
            };
            remote_control_bridge::spawn(state.clone(), bridge_refreshes);
            state
        })
        .layer(axum::middleware::from_fn(
            meta_handlers::request_logging_middleware,
        ))
}

#[cfg(test)]
mod tests;
