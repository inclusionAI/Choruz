//! Lease monitor: detects expired leases and triggers retry or
//! dead-letter handling.

use choruz_session::{PgSessionStore, SessionError};

/// Run the lease monitor loop until cancelled.
pub async fn run_lease_monitor(
    session_store: PgSessionStore,
    lease_interval: std::time::Duration,
    lease_timeout: i64,
) {
    tracing::info!("lease monitor started");
    let mut interval = tokio::time::interval(lease_interval);

    loop {
        interval.tick().await;

        let now = chrono::Utc::now();
        let expired = match session_store.check_expired_leases(now, lease_timeout).await {
            Ok(list) => list,
            Err(e) => {
                tracing::error!(error = %e, "lease monitor: failed to check expired leases");
                continue;
            }
        };

        for exp in &expired {
            tracing::warn!(
                session_key = %exp.session_key,
                command_id = %exp.command_id,
                epoch = exp.epoch,
                attempt = exp.attempt_count,
                max = exp.max_attempts,
                "lease expired"
            );

            if let Err(e) = session_store.handle_lease_expiry(exp).await {
                match e {
                    error @ (SessionError::EpochMismatch { .. }
                    | SessionError::InvalidStateTransition { .. }) => tracing::debug!(
                        command_id = %exp.command_id,
                        error = %error,
                        "lease monitor: expiry was already handled"
                    ),
                    error => tracing::error!(
                        command_id = %exp.command_id,
                        error = %error,
                        "lease monitor: failed to handle expiry"
                    ),
                }
            }
        }
    }
}
