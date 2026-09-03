//! choruz-pipeline: message pipeline runner.
//!
//! Wires together 11 crates into a complete CDC -> Router -> Session ->
//! Executor -> Writer -> Fanout processing chain, running as a single
//! independent process (replacing the retired predecessor daemons
//! daemons that have been retired).
//!
//! # Environment variables
//!
//! See `config.rs` for the full list. Key variables:
//!
//! - `CHORUZ_DATABASE_URL` or `CHORUZ_PG_*` — PostgreSQL connection
//! - `CHORUZ_PIPELINE_METRICS_PORT` — WebSocket fanout HTTP port (default 3020)
//! - `RUST_LOG` — tracing filter (default `info`)

#![allow(
    clippy::items_after_test_module,
    clippy::needless_borrow,
    clippy::needless_return,
    clippy::single_match
)]

mod config;
mod cron_scheduler;
mod dispatch;
mod executor;
mod instructions;
mod lease_monitor;
mod meta;
mod outbox_handler;
mod outbox_watcher;
mod pg_event_source;
mod pg_member_provider;
mod pg_notify;
mod pg_result_store;
mod pipeline;
#[cfg(test)]
mod pipeline_test;
mod retry_scheduler;

#[tokio::main]
async fn main() {
    if let Err(error) = choruz_infrastructure::init_tracing("choruz-pipeline") {
        eprintln!("invalid logging configuration: {error}");
        std::process::exit(2);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("rebootstrap")) {
        // ADR-006 force-rewrite escape hatch. Operators run this when an agent
        // workspace has hand-edited instructions and the auto-refresh skipped
        // it. We dispatch BEFORE booting the pipeline because the command is a
        // one-shot tool, not a long-running service.
        let exit = instructions::run_rebootstrap_command(args.into_iter().skip(1).collect()).await;
        std::process::exit(exit);
    }

    let config = config::PipelineConfig::from_env();
    if let Err(error) = config.validate() {
        tracing::error!(error, "invalid pipeline configuration");
        std::process::exit(2);
    }
    pipeline::run_pipeline(config).await;
}
