//! Pipeline orchestrator: wires together all crate components into
//! a set of cooperating async tasks.
//!
//! Task topology:
//!
//! ```text
//! CDC Poller (choruz-store)
//!   --[mpsc]--> Router loop (choruz-router)
//!                   --[DB writes]--> agent_commands (pending)
//!
//! Dispatch loop (choruz-session + persistent executor)
//!   polls pending commands -> assign lease -> inject into persistent
//!   claude session -> read response from session JSONL
//!   --[mpsc]--> Writer loop (choruz-writer)
//!                   --[DB writes]--> conversation_events (reply)
//!
//! Lease monitor (choruz-session)
//!   polls expired leases -> retry or dead letter
//!
//! Retry scheduler (choruz-session)
//!   polls retry_scheduled commands -> re-dispatch
//!
//! Fanout loop (choruz-fanout + PgEventSource)
//!   polls conversation_events -> pushes to subscribed clients
//!
//! WebSocket fanout HTTP server (choruz-fanout)
//! ```

use std::sync::Arc;

use axum::routing::get;
use choruz_fanout::{FanoutGateway, InMemoryCursorStore};
use choruz_router::{RouterConfig, run_router_loop};
use choruz_session::PgSessionStore;
use choruz_store::{CdcPoller, CdcPollerConfig, EventStore};
use choruz_writer::{AgentResult, WriterConfig, run_writer_loop};
use tokio::sync::mpsc;

use crate::config::PipelineConfig;
use crate::executor::ExecutorContext;
use crate::pg_event_source::PgEventSource;
use crate::pg_member_provider::{PgDecisionSink, PgMemberProvider};
use crate::pg_result_store::PgResultStore;

/// Run the full pipeline until shutdown.
///
/// This spawns all background tasks and waits for any of them to exit
/// (which is treated as a fatal event, triggering shutdown of the rest).
pub async fn run_pipeline(config: PipelineConfig) {
    tracing::info!(?config, "starting choruz-pipeline");

    // -----------------------------------------------------------------------
    // 1. Create shared stores
    // -----------------------------------------------------------------------
    let event_store = EventStore::new(&config.database_url);
    let session_store = PgSessionStore::new(&config.database_url);

    // Verify database connectivity before entering the main loop.
    if let Err(e) = event_store.health_check().await {
        tracing::error!(error = %e, "event store health check failed at startup");
        return;
    }
    if let Err(e) = session_store.health_check().await {
        tracing::error!(error = %e, "session store health check failed at startup");
        return;
    }
    tracing::info!("database connectivity verified");

    // Register this node as an executor.
    if let Err(e) = session_store
        .register_executor(&config.node_id, serde_json::json!({"type": "pipeline"}))
        .await
    {
        tracing::error!(error = %e, "failed to register executor node");
        return;
    }
    tracing::info!(node_id = %config.node_id, "executor node registered");

    // -----------------------------------------------------------------------
    // 1b. Create persistent-session executor with session store reference
    // -----------------------------------------------------------------------
    let session_store_arc = Arc::new(session_store.clone());
    let executor_ctx = Arc::new(
        ExecutorContext::from_config(&config)
            .with_session_store(Arc::clone(&session_store_arc))
            .with_event_store(event_store.clone()),
    );

    // WAL crash recovery: find incomplete turns and mark them as failed
    tracing::info!("running WAL crash recovery...");
    executor_ctx.recover_from_wal().await;

    // -----------------------------------------------------------------------
    // 2. CDC Poller -> mpsc channel -> Router loop
    // -----------------------------------------------------------------------
    let cdc_config = CdcPollerConfig {
        poll_interval: config.cdc_poll_interval(),
        batch_size: config.cdc_batch_size,
        database_url: Some(config.database_url.clone()),
        node_id: config.node_id.clone(),
        claim_lease_duration: std::time::Duration::from_secs(30),
    };
    let poller = CdcPoller::new(event_store.clone(), cdc_config);
    let (cdc_rx, _cdc_handle) = poller.start();

    let member_provider = PgMemberProvider::new(event_store.clone());
    let decision_sink = PgDecisionSink::new(event_store.clone(), session_store.clone());
    let router_config = RouterConfig::default();
    let router_store = event_store.clone();

    let router_task = tokio::spawn(async move {
        run_router_loop(
            cdc_rx,
            router_store,
            member_provider,
            decision_sink,
            router_config,
        )
        .await;
        tracing::warn!("router loop exited");
    });

    // -----------------------------------------------------------------------
    // 3. Dispatch loop: pending commands -> assign lease -> real executor
    //    -> send result to writer channel
    // -----------------------------------------------------------------------
    // Phase 1 simplification: bounded mpsc channel instead of Kafka
    // `agent_results` topic.  The 256-slot buffer provides backpressure:
    // if the writer falls behind, the dispatch loop blocks on send, which
    // naturally throttles command execution.  Upgrade path: replace with a
    // Kafka producer (keyed by conversation_id) and a consumer in the
    // writer task.
    let (writer_tx, writer_rx) = mpsc::channel::<AgentResult>(256);

    let dispatch_session = session_store.clone();
    let dispatch_interval = config.dispatch_interval();
    let dispatch_batch = config.dispatch_batch_size;
    let dispatch_node_id = config.node_id.clone();

    // Set up a LISTEN/NOTIFY wake for the dispatch loop on choruz_commands
    let dispatch_wake = Arc::new(tokio::sync::Notify::new());
    {
        let wake = Arc::clone(&dispatch_wake);
        let db_url = config.database_url.clone();
        tokio::spawn(async move {
            loop {
                match crate::pg_notify::listen_for_commands(&db_url, &wake).await {
                    Ok(()) => break,
                    Err(e) => {
                        tracing::warn!(error = %e, "dispatch LISTEN reconnecting in 2s");
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        });
    }

    let dispatch_executor_ctx = Arc::clone(&executor_ctx);
    let dispatch_task = tokio::spawn(async move {
        crate::dispatch::run_dispatch_loop(
            dispatch_session,
            dispatch_executor_ctx,
            writer_tx.clone(),
            dispatch_wake,
            dispatch_interval,
            dispatch_batch,
            dispatch_node_id,
        )
        .await;
    });

    // -----------------------------------------------------------------------
    // 4. Writer loop: consume AgentResult -> commit reply events
    // -----------------------------------------------------------------------
    let result_store = PgResultStore::new(event_store.clone(), session_store.clone());
    let writer_config = WriterConfig::default();

    let writer_task = tokio::spawn(async move {
        run_writer_loop(writer_rx, result_store, writer_config).await;
        tracing::warn!("writer loop exited");
    });

    // -----------------------------------------------------------------------
    // 5. Lease monitor: detect expired leases and handle them
    // -----------------------------------------------------------------------
    let lease_session = session_store.clone();
    let lease_interval = config.lease_check_interval();
    let lease_timeout = config.lease_timeout_secs;

    let lease_task = tokio::spawn(async move {
        crate::lease_monitor::run_lease_monitor(lease_session, lease_interval, lease_timeout).await;
    });

    // -----------------------------------------------------------------------
    // 6. Retry scheduler: find retriable commands and re-dispatch them
    // -----------------------------------------------------------------------
    let retry_session = session_store.clone();
    let retry_interval = config.retry_check_interval();
    let retry_batch = config.retry_batch_size;

    let retry_task = tokio::spawn(async move {
        crate::retry_scheduler::run_retry_loop(retry_session, retry_interval, retry_batch).await;
    });

    // -----------------------------------------------------------------------
    // 6b. Cron scheduler: checks due cron jobs and dispatches them
    // -----------------------------------------------------------------------
    let cron_event_store = event_store.clone();
    let cron_session_store = session_store.clone();
    let cron_task = tokio::spawn(async move {
        crate::cron_scheduler::run_cron_scheduler(
            cron_event_store,
            cron_session_store,
            std::time::Duration::from_secs(30),
        )
        .await;
    });

    // -----------------------------------------------------------------------
    // 6c. Maildir outbox watcher: drains .choruz-outbox/new/ for non-headless
    //     (PTY/terminal-mode) bindings. Headless agents already have their
    //     outbox drained by the executor after each turn; PTY agents have
    //     nobody else watching since the runner was retired.
    // -----------------------------------------------------------------------
    let watcher_runtime = Arc::new(choruz_agent_runtime::RuntimeStore::new(
        &config.database_url,
    ));
    let watcher_event_store = event_store.clone();
    let watcher_gateway_base = config.gateway_base_url.clone();
    let outbox_watcher_task = tokio::spawn(async move {
        crate::outbox_watcher::run_outbox_watcher_loop(
            watcher_runtime,
            watcher_event_store,
            watcher_gateway_base,
            std::time::Duration::from_secs(2),
        )
        .await;
    });

    // -----------------------------------------------------------------------
    // 7. Fanout loop: real PgEventSource backed by conversation_events table
    // -----------------------------------------------------------------------
    let fanout_source = PgEventSource::new(event_store.clone());
    let fanout_cursors = InMemoryCursorStore::new();
    let fanout_gateway = Arc::new(FanoutGateway::new(fanout_source, fanout_cursors));

    let fanout_gw_loop = Arc::clone(&fanout_gateway);
    let fanout_task = tokio::spawn(async move {
        fanout_gw_loop
            .run_fanout_loop(std::time::Duration::from_secs(2))
            .await;
    });

    // -----------------------------------------------------------------------
    // 8. WebSocket fanout HTTP server
    // -----------------------------------------------------------------------
    let ws_host = config.metrics_host.clone();
    let ws_port = config.metrics_port;

    let ws_state = choruz_fanout::WsFanoutState {
        gateway: Arc::clone(&fanout_gateway),
    };
    let readiness_event_store = event_store.clone();
    let readiness_session_store = session_store.clone();
    let ws_routes = choruz_fanout::ws_fanout_routes(ws_state)
        .route("/healthz", get(crate::meta::liveness))
        .route("/metrics", get(crate::meta::metrics))
        .route(
            "/readyz",
            get(move || {
                crate::meta::readiness(
                    readiness_event_store.clone(),
                    readiness_session_store.clone(),
                )
            }),
        );

    let ws_task = tokio::spawn(async move {
        let addr = format!("{ws_host}:{ws_port}");
        tracing::info!(%addr, "ws fanout server starting");

        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(error = %e, %addr, "failed to bind ws server");
                return;
            }
        };

        if let Err(e) = axum::serve(listener, ws_routes.into_make_service()).await {
            tracing::error!(error = %e, "ws server error");
        }
    });

    // -----------------------------------------------------------------------
    // 9. Wait for any task to exit (fatal)
    // -----------------------------------------------------------------------
    tracing::info!(
        "choruz-pipeline running: cdc_poller + router + dispatch + writer + lease_monitor + retry + cron + outbox_watcher + fanout(pg) + ws"
    );

    tokio::select! {
        r = router_task => {
            tracing::error!(?r, "router task exited");
        }
        r = dispatch_task => {
            tracing::error!(?r, "dispatch task exited");
        }
        r = writer_task => {
            tracing::error!(?r, "writer task exited");
        }
        r = lease_task => {
            tracing::error!(?r, "lease monitor task exited");
        }
        r = retry_task => {
            tracing::error!(?r, "retry scheduler task exited");
        }
        r = cron_task => {
            tracing::error!(?r, "cron scheduler task exited");
        }
        r = outbox_watcher_task => {
            tracing::error!(?r, "outbox watcher task exited");
        }
        r = fanout_task => {
            tracing::error!(?r, "fanout task exited");
        }
        r = ws_task => {
            tracing::error!(?r, "ws server task exited");
        }
    }

    // Graceful shutdown: terminate all persistent sessions
    tracing::info!("shutting down persistent executor sessions...");
    executor_ctx.shutdown_all().await;

    tracing::warn!("choruz-pipeline shutting down");
}

#[cfg(test)]
mod tests {
    /// Truncate a prompt string (utility used in tests).
    fn truncate_prompt(s: &str, max: usize) -> &str {
        if s.len() <= max { s } else { &s[..max] }
    }

    #[test]
    fn truncate_prompt_short() {
        assert_eq!(truncate_prompt("hello", 10), "hello");
    }

    #[test]
    fn truncate_prompt_long() {
        let long = "a".repeat(300);
        let result = truncate_prompt(&long, 200);
        assert_eq!(result.len(), 200);
    }
}
