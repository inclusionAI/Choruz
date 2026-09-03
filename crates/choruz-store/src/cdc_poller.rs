//! CDC poller: claims unpublished outbox entries and dispatches them to an
//! in-memory channel.
//!
//! Uses PostgreSQL LISTEN/NOTIFY for instant wake-up on new entries, with
//! a fallback poll interval for reliability (missed notifications, reconnects).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::{Notify, mpsc};
use tracing;

use crate::EventStore;
use crate::event_outbox::OutboxRow;

/// A published outbox entry forwarded to consumers.
pub type PublishedEvent = OutboxRow;

/// Configuration for the CDC poller.
#[derive(Debug, Clone)]
pub struct CdcPollerConfig {
    /// Fallback poll interval — safety net for missed notifications.
    pub poll_interval: Duration,
    /// Maximum number of entries to claim per poll cycle.
    pub batch_size: i64,
    /// Database URL for the dedicated LISTEN connection.
    pub database_url: Option<String>,
    /// Unique ID for this poller node.
    pub node_id: String,
    /// Duration for which a claim lease is valid.
    pub claim_lease_duration: Duration,
}

impl Default for CdcPollerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(5),
            batch_size: 100,
            database_url: None,
            node_id: "default-node".to_string(),
            claim_lease_duration: Duration::from_secs(30),
        }
    }
}

/// CDC poller that claims unpublished outbox entries and sends them to
/// an mpsc channel. Uses LISTEN/NOTIFY for instant wake-up.
pub struct CdcPoller {
    store: EventStore,
    config: CdcPollerConfig,
    running: Arc<AtomicBool>,
}

impl CdcPoller {
    pub fn new(store: EventStore, config: CdcPollerConfig) -> Self {
        Self {
            store,
            config,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Start the poller in a background tokio task.
    pub fn start(self) -> (mpsc::Receiver<PublishedEvent>, CdcPollerHandle) {
        let (tx, rx) = mpsc::channel::<PublishedEvent>(1024);
        let running = Arc::clone(&self.running);
        running.store(true, Ordering::SeqCst);

        let handle_running = Arc::clone(&running);
        let store = self.store;
        let config = self.config;

        // Shared wake signal
        let wake = Arc::new(Notify::new());

        // Spawn LISTEN task if database_url is provided
        if let Some(ref db_url) = config.database_url {
            let wake_clone = Arc::clone(&wake);
            let url = db_url.clone();
            tokio::spawn(async move {
                loop {
                    match run_listener(&url, &wake_clone).await {
                        Ok(()) => break,
                        Err(e) => {
                            tracing::warn!(error = %e, "LISTEN connection lost, reconnecting in 2s");
                            tokio::time::sleep(Duration::from_secs(2)).await;
                        }
                    }
                }
            });
        } else {
            tracing::info!("CDC poller: no database_url for LISTEN, using fallback polling only");
        }

        // Main poller loop
        tokio::spawn(async move {
            let mut fallback = tokio::time::interval(config.poll_interval);

            while running.load(Ordering::SeqCst) {
                tokio::select! {
                    _ = wake.notified() => {}
                    _ = fallback.tick() => {}
                }

                match store
                    .claim_unpublished_entries(
                        config.batch_size,
                        &config.node_id,
                        chrono::Duration::from_std(config.claim_lease_duration).unwrap(),
                    )
                    .await
                {
                    Ok(batch) if batch.is_empty() => {}
                    Ok(batch) => {
                        let count = batch.len();
                        let mut send_ok = true;
                        for entry in batch {
                            if tx.send(entry).await.is_err() {
                                tracing::warn!("CDC poller: receiver dropped, stopping");
                                send_ok = false;
                                break;
                            }
                        }
                        if send_ok {
                            tracing::debug!(count, "CDC poller: dispatched entries");
                        } else {
                            running.store(false, Ordering::SeqCst);
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "CDC poller: failed to claim outbox entries");
                    }
                }
            }
            tracing::info!("CDC poller stopped");
        });

        (
            rx,
            CdcPollerHandle {
                running: handle_running,
            },
        )
    }
}

/// Run a dedicated LISTEN connection using raw tokio_postgres (not pooled).
/// Wakes the poller instantly on each NOTIFY.
async fn run_listener(database_url: &str, wake: &Notify) -> Result<(), String> {
    run_listener_with_ready(database_url, wake, None).await
}

async fn run_listener_with_ready(
    database_url: &str,
    wake: &Notify,
    ready: Option<tokio::sync::oneshot::Sender<()>>,
) -> Result<(), String> {
    let pg_config: tokio_postgres::Config = database_url
        .parse()
        .map_err(|e| format!("parse error: {e}"))?;

    let (client, mut connection) = pg_config
        .connect(tokio_postgres::NoTls)
        .await
        .map_err(|e| format!("connect error: {e}"))?;

    // The connection must be driven in a background task.
    // We use the connection's poll_message stream to receive notifications.
    let (conn_tx, mut conn_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        // Process connection messages — notifications come through here
        loop {
            match futures_util::future::poll_fn(|cx| connection.poll_message(cx)).await {
                Some(Ok(msg)) => {
                    if let tokio_postgres::AsyncMessage::Notification(_n) = msg {
                        let _ = conn_tx.send(());
                    }
                }
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "LISTEN connection error");
                    break;
                }
                None => break,
            }
        }
    });

    // Subscribe to channels
    client
        .execute("LISTEN choruz_outbox", &[])
        .await
        .map_err(|e| format!("LISTEN choruz_outbox: {e}"))?;
    client
        .execute("LISTEN choruz_commands", &[])
        .await
        .map_err(|e| format!("LISTEN choruz_commands: {e}"))?;

    tracing::info!("CDC poller: LISTEN active (choruz_outbox + choruz_commands)");
    if let Some(ready) = ready {
        let _ = ready.send(());
    }

    // Wait for notifications and wake the poller
    while conn_rx.recv().await.is_some() {
        wake.notify_one();
    }

    Err("LISTEN connection closed".into())
}

/// Handle to control a running CDC poller.
pub struct CdcPollerHandle {
    running: Arc<AtomicBool>,
}

impl CdcPollerHandle {
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod listener_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::sync::{Notify, oneshot};
    use tokio_postgres::NoTls;

    use super::run_listener_with_ready;

    async fn notify(database_url: &str, channel: &str) {
        let (client, connection) = tokio_postgres::connect(database_url, NoTls)
            .await
            .expect("connect notification sender");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
            .execute(&format!("NOTIFY {channel}"), &[])
            .await
            .expect("send notification");
    }

    #[tokio::test]
    async fn choruz_listener_wakes_for_new_channels_but_not_legacy_channels() {
        let Ok(database_url) = std::env::var("CHORUZ_LISTENER_TEST_DATABASE_URL") else {
            return;
        };
        let wake = Arc::new(Notify::new());
        let (ready_tx, ready_rx) = oneshot::channel();
        let task = tokio::spawn({
            let database_url = database_url.clone();
            let wake = Arc::clone(&wake);
            async move { run_listener_with_ready(&database_url, &wake, Some(ready_tx)).await }
        });
        tokio::time::timeout(Duration::from_secs(1), ready_rx)
            .await
            .expect("CDC listener becomes ready")
            .expect("CDC listener readiness sender is alive");

        notify(&database_url, "echat_outbox").await;
        assert!(
            tokio::time::timeout(Duration::from_millis(200), wake.notified())
                .await
                .is_err()
        );

        notify(&database_url, "choruz_outbox").await;
        tokio::time::timeout(Duration::from_secs(1), wake.notified())
            .await
            .expect("choruz outbox notification wakes listener");

        notify(&database_url, "echat_commands").await;
        assert!(
            tokio::time::timeout(Duration::from_millis(200), wake.notified())
                .await
                .is_err()
        );
        notify(&database_url, "choruz_commands").await;
        tokio::time::timeout(Duration::from_secs(1), wake.notified())
            .await
            .expect("choruz command notification wakes listener");
        task.abort();
    }
}
