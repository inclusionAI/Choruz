//! PostgreSQL LISTEN/NOTIFY helper for instant dispatch wake-up on the
//! `choruz_commands` channel.

/// LISTEN on choruz_commands channel for instant dispatch wake-up.
pub async fn listen_for_commands(
    database_url: &str,
    wake: &tokio::sync::Notify,
) -> Result<(), String> {
    listen_for_commands_with_ready(database_url, wake, None).await
}

async fn listen_for_commands_with_ready(
    database_url: &str,
    wake: &tokio::sync::Notify,
    ready: Option<tokio::sync::oneshot::Sender<()>>,
) -> Result<(), String> {
    let mut pg_config: tokio_postgres::Config =
        database_url.parse().map_err(|e| format!("parse: {e}"))?;

    // TCP keepalives so the LISTEN connection survives PG's idle timeout
    // (default ~5 min). Without this the connection is reaped every few
    // minutes, each break drops NOTIFYs for ~2s until auto-reconnect and the
    // 500ms CDC poll has to cover the gap — visible as a dispatch latency
    // spike. Keepalive every 60s keeps the socket warm indefinitely.
    pg_config.keepalives(true);
    pg_config.keepalives_idle(std::time::Duration::from_secs(60));

    let (client, mut connection) = pg_config
        .connect(tokio_postgres::NoTls)
        .await
        .map_err(|e| format!("connect: {e}"))?;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            match futures_util::future::poll_fn(|cx| connection.poll_message(cx)).await {
                Some(Ok(msg)) => {
                    if let tokio_postgres::AsyncMessage::Notification(_) = msg {
                        let _ = tx.send(());
                    }
                }
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "dispatch LISTEN error");
                    break;
                }
                None => break,
            }
        }
    });

    client
        .execute("LISTEN choruz_commands", &[])
        .await
        .map_err(|e| format!("LISTEN: {e}"))?;

    tracing::info!("dispatch: LISTEN choruz_commands active");
    if let Some(ready) = ready {
        let _ = ready.send(());
    }

    while rx.recv().await.is_some() {
        wake.notify_one();
    }

    Err("dispatch LISTEN closed".into())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::sync::{Notify, oneshot};
    use tokio_postgres::NoTls;

    use super::listen_for_commands_with_ready;

    #[tokio::test]
    async fn command_listener_wakes_for_choruz_channel_not_legacy_channel() {
        let Ok(database_url) = std::env::var("CHORUZ_LISTENER_TEST_DATABASE_URL") else {
            return;
        };
        let wake = Arc::new(Notify::new());
        let (ready_tx, ready_rx) = oneshot::channel();
        let task = tokio::spawn({
            let database_url = database_url.clone();
            let wake = Arc::clone(&wake);
            async move { listen_for_commands_with_ready(&database_url, &wake, Some(ready_tx)).await }
        });
        tokio::time::timeout(Duration::from_secs(1), ready_rx)
            .await
            .expect("dispatch listener becomes ready")
            .expect("dispatch listener readiness sender is alive");

        let (client, connection) = tokio_postgres::connect(&database_url, NoTls)
            .await
            .expect("connect notification sender");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client.execute("NOTIFY echat_commands", &[]).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(200), wake.notified())
                .await
                .is_err()
        );
        client.execute("NOTIFY choruz_commands", &[]).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), wake.notified())
            .await
            .expect("choruz command notification wakes dispatch listener");
        task.abort();
    }
}
