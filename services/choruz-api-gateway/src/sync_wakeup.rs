use std::{sync::Arc, time::Duration};

use futures_util::future::poll_fn;
use tokio::sync::broadcast;

const GLOBAL_WAKEUP: &str = "*";

#[derive(Clone)]
pub(crate) struct SyncWakeupHub {
    inner: Arc<SyncWakeupInner>,
}

struct SyncWakeupInner {
    tx: broadcast::Sender<String>,
    ready: tokio::sync::watch::Receiver<bool>,
    listener: tokio::task::AbortHandle,
}

impl Drop for SyncWakeupInner {
    fn drop(&mut self) {
        self.listener.abort();
    }
}

impl SyncWakeupHub {
    pub(crate) fn spawn(database_url: String) -> Self {
        let (tx, _) = broadcast::channel(1024);
        let (ready_tx, ready_rx) = tokio::sync::watch::channel(false);
        let listener_tx = tx.clone();
        let task = tokio::spawn(async move {
            let mut backoff = Duration::from_millis(100);
            loop {
                let _ = ready_tx.send(false);
                if let Err(error) = listen_once(&database_url, &listener_tx, &ready_tx).await {
                    tracing::warn!(%error, "dashboard sync LISTEN connection closed");
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(5));
                // Every socket re-reads its durable cursor after a listener
                // outage. The notification itself never carries state.
                let _ = listener_tx.send(GLOBAL_WAKEUP.into());
            }
        });
        Self {
            inner: Arc::new(SyncWakeupInner {
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
            Err("sync wakeup listener stopped".into())
        })
        .await
        .map_err(|_| "sync wakeup listener startup timed out".to_owned())?
    }
}

async fn listen_once(
    database_url: &str,
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
        .batch_execute("LISTEN choruz_sync_change")
        .await
        .map_err(|error| format!("LISTEN: {error}"))?;
    let _ = ready.send(true);
    // Close the startup race: changes committed before LISTEN became active
    // are recovered by every connected socket from the durable table.
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
