use std::{sync::Arc, time::Duration};

use futures_util::future::poll_fn;
use tokio::sync::broadcast;

const GLOBAL_WAKEUP: &str = "*";

/// Dashboard sync feed changes; the payload is the affected principal.
pub(crate) const SYNC_CHANNEL: &str = "choruz_sync_change";
/// Background work that became due; the payload names the worker.
pub(crate) const WORKER_CHANNEL: &str = "choruz_background_work";

/// One PostgreSQL LISTEN connection fanned out to in-process subscribers.
/// A notification never carries state: subscribers re-read durable rows, and
/// every reconnect broadcasts `*` so nothing committed meanwhile is missed.
#[derive(Clone)]
pub(crate) struct WakeupHub {
    inner: Arc<WakeupInner>,
}

struct WakeupInner {
    tx: broadcast::Sender<String>,
    ready: tokio::sync::watch::Receiver<bool>,
    listener: tokio::task::AbortHandle,
}

impl Drop for WakeupInner {
    fn drop(&mut self) {
        self.listener.abort();
    }
}

impl WakeupHub {
    pub(crate) fn spawn(database_url: String, channel: &'static str) -> Self {
        let (tx, _) = broadcast::channel(1024);
        let (ready_tx, ready_rx) = tokio::sync::watch::channel(false);
        let listener_tx = tx.clone();
        let task = tokio::spawn(async move {
            let mut backoff = Duration::from_millis(100);
            loop {
                let _ = ready_tx.send(false);
                if let Err(error) =
                    listen_once(&database_url, channel, &listener_tx, &ready_tx).await
                {
                    tracing::warn!(%error, channel, "LISTEN connection closed");
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(5));
                // Every socket re-reads its durable cursor after a listener
                // outage. The notification itself never carries state.
                let _ = listener_tx.send(GLOBAL_WAKEUP.into());
            }
        });
        Self {
            inner: Arc::new(WakeupInner {
                tx,
                ready: ready_rx,
                listener: task.abort_handle(),
            }),
        }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<String> {
        self.inner.tx.subscribe()
    }

    pub(crate) async fn wait_ready(&self) -> Result<(), String> {
        let mut ready = self.inner.ready.clone();
        if *ready.borrow() {
            return Ok(());
        }
        tokio::time::timeout(Duration::from_secs(5), async move {
            while ready.changed().await.is_ok() {
                if *ready.borrow() {
                    return Ok(());
                }
            }
            Err("wakeup listener stopped".into())
        })
        .await
        .map_err(|_| "wakeup listener startup timed out".to_owned())?
    }
}

async fn listen_once(
    database_url: &str,
    channel: &str,
    tx: &broadcast::Sender<String>,
    ready: &tokio::sync::watch::Sender<bool>,
) -> Result<(), String> {
    let config: tokio_postgres::Config = database_url
        .parse()
        .map_err(|error| format!("parse database URL: {error}"))?;
    let (client, mut connection) = config
        .connect(tokio_postgres::NoTls)
        .await
        .map_err(|error| format!("connect: {error}"))?;
    let (notification_tx, mut notification_rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            match poll_fn(|cx| connection.poll_message(cx)).await {
                Some(Ok(tokio_postgres::AsyncMessage::Notification(notification))) => {
                    if notification_tx
                        .send(Ok(notification.payload().to_owned()))
                        .is_err()
                    {
                        break;
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    let _ = notification_tx.send(Err(format!("receive: {error}")));
                    break;
                }
                None => {
                    let _ = notification_tx.send(Err("connection ended".into()));
                    break;
                }
            }
        }
    });
    client
        .batch_execute(&format!("LISTEN {channel}"))
        .await
        .map_err(|error| format!("LISTEN: {error}"))?;
    let _ = ready.send(true);
    // Close the startup race: changes committed before LISTEN became active
    // are recovered by every subscriber from the durable table.
    let _ = tx.send(GLOBAL_WAKEUP.into());

    while let Some(notification) = notification_rx.recv().await {
        match notification {
            Ok(payload) => {
                let _ = tx.send(payload);
            }
            Err(error) => return Err(error),
        }
    }
    Err("notification channel ended".into())
}

pub(crate) fn is_relevant_wakeup(payload: &str, principal_id: &str) -> bool {
    payload == GLOBAL_WAKEUP || payload == principal_id
}

/// Wait until a background worker should look for work: its poll interval
/// elapsed (`false`), or a `WORKER_CHANNEL` notification named it (`true`).
/// A lagged receiver may have missed one, so it counts as notified; a closed
/// hub leaves the worker polling.
pub(crate) async fn next_round(
    interval: &mut tokio::time::Interval,
    wakeups: &mut broadcast::Receiver<String>,
    worker: &str,
) -> bool {
    loop {
        tokio::select! {
            _ = interval.tick() => return false,
            received = wakeups.recv() => match received {
                Ok(payload) if is_relevant_wakeup(&payload, worker) => return true,
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => return true,
                Err(broadcast::error::RecvError::Closed) => {
                    interval.tick().await;
                    return false;
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_worker_wakes_for_its_own_notification_or_its_poll() {
        let (tx, mut rx) = broadcast::channel(8);
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        // The first tick is immediate: a worker looks for work when it starts.
        assert!(!next_round(&mut interval, &mut rx, "learning").await);

        tx.send("evaluation".to_owned()).unwrap();
        tx.send("learning".to_owned()).unwrap();
        let started = tokio::time::Instant::now();
        assert!(next_round(&mut interval, &mut rx, "learning").await);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "another worker's notification must not end the wait"
        );

        tx.send(GLOBAL_WAKEUP.to_owned()).unwrap();
        assert!(next_round(&mut interval, &mut rx, "learning").await);

        assert!(!next_round(&mut interval, &mut rx, "learning").await);
        assert!(
            started.elapsed() >= Duration::from_secs(15),
            "without a notification the poll interval still applies"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_lagged_worker_rechecks_at_once_and_a_closed_hub_falls_back_to_polling() {
        let (tx, mut rx) = broadcast::channel(2);
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        assert!(!next_round(&mut interval, &mut rx, "learning").await);

        // More notifications than the channel holds: the worker may have
        // missed its own, so it looks for work immediately.
        for _ in 0..5 {
            tx.send("evaluation".to_owned()).unwrap();
        }
        let started = tokio::time::Instant::now();
        assert!(next_round(&mut interval, &mut rx, "learning").await);
        assert!(started.elapsed() < Duration::from_secs(1));

        // Without a listener the worker neither spins nor stops: each round
        // waits for the next poll.
        drop(tx);
        for _ in 0..3 {
            let round = tokio::time::Instant::now();
            assert!(!next_round(&mut interval, &mut rx, "learning").await);
            assert!(
                round.elapsed() >= Duration::from_secs(15),
                "every round after the hub closes waits for the poll; it must not busy-loop"
            );
        }
    }
}
