//! Per-binding Maildir outbox watcher.
//!
//! Periodically scans every active runtime binding's
//! `<workspace>/.choruz-outbox/new/` directory and dispatches any pending
//! command files via [`crate::outbox_handler::process_outbox_commands`].
//!
//! # Why this exists
//!
//! Pipeline's executor already drains outboxes for headless `--print` agents
//! after each turn (`executor.rs:1050`). But `claude_terminal` / PTY-mode
//! bindings and external `webhook_agent` bindings do their work outside that
//! headless completion path. Their runtimes can still write outbox commands
//! as side effects of agent work. With nobody scanning those workspaces the
//! commands accumulate forever.
//!
//! The retired runner used to poll these (with kqueue/inotify); since it was
//! disabled on 2026-04-02 (commit 32a230b), PTY-mode `provision_agent` /
//! `create_group` / `send` commands have been silently dropped. This task
//! restores that path inside pipeline, which is the natural owner for
//! agent-side workflow processing.
//!
//! # Strategy
//!
//! Polling, simple version. Runner had a `notify` (fs-event) watcher; we
//! can upgrade later if 2-second poll latency matters. The hot path is
//! `read_dir` on N small workspaces — cheap.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use choruz_agent_runtime::{DriverType, RuntimeStore};
use choruz_store::EventStore;

pub async fn run_outbox_watcher_loop(
    runtime_store: Arc<RuntimeStore>,
    event_store: EventStore,
    gateway_base_url: String,
    interval: Duration,
) {
    tracing::info!(?interval, %gateway_base_url, "outbox watcher started");
    loop {
        match runtime_store.list_active_bindings().await {
            Ok(bindings) => {
                let active_headless_agents = match load_active_headless_agents(&event_store).await {
                    Ok(agent_ids) => agent_ids,
                    Err(error) => {
                        tracing::warn!(%error, "outbox watcher: active headless lookup failed");
                        tokio::time::sleep(interval).await;
                        continue;
                    }
                };
                for binding in bindings {
                    if !should_drain_binding(
                        &binding.driver_type,
                        &binding.agent_principal_id,
                        &active_headless_agents,
                    ) {
                        continue;
                    }
                    if binding.workspace_path.is_empty() {
                        continue;
                    }
                    let work_dir = PathBuf::from(&binding.workspace_path);
                    let maildir_new = work_dir.join(".choruz-outbox").join("new");
                    if !maildir_new.is_dir() {
                        continue;
                    }
                    // Cheap pre-check: skip the cookie+http overhead inside
                    // process_outbox_commands when there's nothing to do.
                    let has_files = std::fs::read_dir(&maildir_new)
                        .map(|mut it| it.any(|e| e.is_ok()))
                        .unwrap_or(false);
                    if !has_files {
                        continue;
                    }

                    let session_key = format!("watcher:{}", binding.id);
                    // PTY/webhook bindings do not flow through the headless
                    // executor writer, so visible replies/errors from outbox
                    // commands are published directly to the bound
                    // conversation here. Side-effect commands still hit
                    // gateway/web directly inside the handler.
                    let result = crate::outbox_handler::process_outbox_commands_with_stats(
                        &session_key,
                        &binding.agent_principal_id,
                        &work_dir,
                        &gateway_base_url,
                        Some(&event_store),
                    )
                    .await;
                    if !result.command_results.is_empty() {
                        // The handler already persists non-chat command
                        // results under `.choruz-outbox/results/`; PTY watcher
                        // mode has no writer result channel, so keep them out
                        // of the timeline and surface the durable result path
                        // plus telemetry here.
                        tracing::info!(
                            binding_id = %binding.id,
                            agent_id = %binding.agent_principal_id,
                            command_results = ?result.command_results,
                            "outbox watcher: processed non-chat command results"
                        );
                    }
                    if !result.reply.trim().is_empty() {
                        publish_watcher_reply(
                            &event_store,
                            &session_key,
                            &binding.conversation_id,
                            &binding.agent_principal_id,
                            &result.reply,
                        )
                        .await;
                    }

                    tracing::debug!(
                        binding_id = %binding.id,
                        agent_id = %binding.agent_principal_id,
                        workspace = %binding.workspace_path,
                        "outbox watcher: drained maildir"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "outbox watcher: list_active_bindings failed");
            }
        }
        tokio::time::sleep(interval).await;
    }
}

fn drains_via_watcher(driver_type: &DriverType) -> bool {
    matches!(
        driver_type,
        DriverType::ClaudeTerminal
            | DriverType::CodexTerminal
            | DriverType::PiTerminal
            | DriverType::GrokTerminal
            | DriverType::OpenCodeTerminal
            | DriverType::WebhookAgent
    )
}

fn should_drain_binding(
    driver_type: &DriverType,
    agent_id: &str,
    active_headless_agents: &HashSet<String>,
) -> bool {
    drains_via_watcher(driver_type) && !active_headless_agents.contains(agent_id)
}

async fn load_active_headless_agents(event_store: &EventStore) -> Result<HashSet<String>, String> {
    let client = event_store
        .connect()
        .await
        .map_err(|error| error.to_string())?;
    let rows = client
        .query(
            "SELECT DISTINCT agent_id
             FROM agent_commands
             WHERE status IN ('leased', 'started', 'heartbeating')",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(rows.into_iter().map(|row| row.get(0)).collect())
}

async fn publish_watcher_reply(
    event_store: &EventStore,
    session_key: &str,
    conversation_id: &str,
    agent_id: &str,
    content: &str,
) {
    let mut client = match event_store.connect().await {
        Ok(client) => client,
        Err(e) => {
            tracing::warn!(session_key, error = %e, "outbox watcher: reply DB connect failed");
            return;
        }
    };
    let tx = match client.transaction().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::warn!(session_key, error = %e, "outbox watcher: reply tx begin failed");
            return;
        }
    };

    if let Err(e) = tx
        .execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&conversation_id],
        )
        .await
    {
        tracing::warn!(session_key, conversation_id, error = %e, "outbox watcher: reply lock failed");
        tx.rollback().await.ok();
        return;
    }

    let active_member = match tx
        .query_opt(
            "SELECT 1
             FROM conversation c
             JOIN principal p ON p.id = $2
             JOIN conversation_member cm
               ON cm.conv_id = c.id
              AND cm.principal_id = p.id
              AND cm.removed_at IS NULL
             WHERE c.id = $1
               AND c.workspace_id = p.workspace_id
               AND p.deleted_at IS NULL
             LIMIT 1",
            &[&conversation_id, &agent_id],
        )
        .await
    {
        Ok(row) => row.is_some(),
        Err(e) => {
            tracing::warn!(session_key, conversation_id, agent_id, error = %e, "outbox watcher: membership check failed");
            tx.rollback().await.ok();
            return;
        }
    };
    if !active_member {
        tracing::warn!(
            session_key,
            conversation_id,
            agent_id,
            "outbox watcher: reply denied because binding actor is not an active conversation member"
        );
        tx.rollback().await.ok();
        return;
    }

    let message_id = choruz_ids::MessageId::new().to_string();
    let event_type = "message";
    let content_type = "text/plain";
    let metadata = serde_json::json!({
        "source": "outbox_watcher",
    });
    let content_opt: Option<&str> = Some(content);
    let client_msg_id: Option<&str> = None;
    let turn_id: Option<&str> = None;
    let reply_event_id: Option<&str> = None;
    let seq: i64 = match tx
        .query_one(
            "INSERT INTO conversation_events
                (conversation_id, seq, event_id, event_type, sender_id,
                 content, content_type, metadata, client_msg_id, turn_id,
                 reply_event_id, created_at)
             VALUES (
                $1,
                COALESCE((SELECT MAX(seq) FROM conversation_events WHERE conversation_id = $1), 0) + 1,
                $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW()
             )
             RETURNING seq",
            &[
                &conversation_id,
                &message_id,
                &event_type,
                &agent_id,
                &content_opt,
                &content_type,
                &metadata,
                &client_msg_id,
                &turn_id,
                &reply_event_id,
            ],
        )
        .await
    {
        Ok(row) => row.get(0),
        Err(e) => {
            tracing::warn!(session_key, conversation_id, error = %e, "outbox watcher: reply insert failed");
            tx.rollback().await.ok();
            return;
        }
    };

    let payload = serde_json::json!({
        "message_id": message_id,
        "conversation_id": conversation_id,
        "sender_id": agent_id,
        "content": content,
        "content_type": content_type,
        "seq": seq,
        "metadata": metadata,
    });
    if let Err(e) = tx
        .execute(
            "INSERT INTO event_outbox
                (aggregate_type, aggregate_id, event_type, payload, created_at, published)
             VALUES ('conversation_event', $1, 'message', $2, NOW(), FALSE)",
            &[&conversation_id, &payload],
        )
        .await
    {
        tracing::warn!(session_key, conversation_id, error = %e, "outbox watcher: reply outbox insert failed");
        tx.rollback().await.ok();
        return;
    }

    if let Err(e) = tx
        .execute(
            "UPDATE conversation SET total_msg_count = total_msg_count + 1 WHERE id = $1",
            &[&conversation_id],
        )
        .await
    {
        tracing::warn!(session_key, conversation_id, error = %e, "outbox watcher: reply count update failed");
        tx.rollback().await.ok();
        return;
    }

    if let Err(e) = tx.commit().await {
        tracing::warn!(session_key, conversation_id, error = %e, "outbox watcher: reply commit failed");
        return;
    }

    tracing::info!(
        session_key,
        conversation_id,
        agent_id,
        seq,
        "outbox watcher: published command reply"
    );
}

#[cfg(test)]
mod tests {
    use super::{drains_via_watcher, publish_watcher_reply, should_drain_binding};
    use choruz_agent_runtime::DriverType;
    use choruz_store::EventStore;
    use std::collections::HashSet;
    use tokio_postgres::NoTls;

    #[test]
    fn watcher_drains_external_runtime_bindings() {
        assert!(drains_via_watcher(&DriverType::ClaudeTerminal));
        assert!(drains_via_watcher(&DriverType::CodexTerminal));
        assert!(drains_via_watcher(&DriverType::PiTerminal));
        assert!(drains_via_watcher(&DriverType::GrokTerminal));
        assert!(drains_via_watcher(&DriverType::OpenCodeTerminal));
        assert!(drains_via_watcher(&DriverType::WebhookAgent));
        assert!(!drains_via_watcher(&DriverType::ClaudePrint));
        assert!(!drains_via_watcher(&DriverType::CodexExec));
    }

    #[test]
    fn skips_terminal_binding_while_headless_turn_owns_its_outbox() {
        let active = HashSet::from(["agent-1".to_owned()]);

        assert!(!should_drain_binding(
            &DriverType::CodexTerminal,
            "agent-1",
            &active,
        ));

        assert!(should_drain_binding(
            &DriverType::CodexTerminal,
            "agent-2",
            &active,
        ));
    }

    #[tokio::test]
    async fn watcher_reply_is_written_to_bound_conversation() {
        let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
            return;
        };

        let workspace_id = choruz_common::new_id();
        let agent_id = choruz_common::new_id();
        let conversation_id = choruz_common::new_id();
        let content = format!("visible watcher error {}", choruz_common::new_id());

        let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("connect for watcher reply test");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
            .execute(
                "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'agent', 'Watcher Agent', FALSE, NOW(), NOW())",
                &[&agent_id, &workspace_id],
            )
            .await
            .expect("seed watcher agent principal");
        client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'direct', NULL, $3, NOW(), NOW())",
                &[&conversation_id, &workspace_id, &agent_id],
            )
            .await
            .expect("seed watcher conversation");
        client
            .execute(
                "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, NOW())",
                &[&conversation_id, &agent_id],
            )
            .await
            .expect("seed watcher membership");

        let store = EventStore::new(&db_url);
        publish_watcher_reply(
            &store,
            "watcher:test",
            &conversation_id,
            &agent_id,
            &content,
        )
        .await;

        let row = client
            .query_one(
                "SELECT sender_id, content, metadata->>'source' AS source
                 FROM conversation_events
                 WHERE conversation_id = $1 AND content = $2",
                &[&conversation_id, &Some(content.as_str())],
            )
            .await
            .expect("watcher reply event exists");
        assert_eq!(row.get::<_, String>("sender_id"), agent_id);
        assert_eq!(
            row.get::<_, Option<String>>("content").as_deref(),
            Some(content.as_str())
        );
        assert_eq!(
            row.get::<_, Option<String>>("source").as_deref(),
            Some("outbox_watcher")
        );
    }

    #[tokio::test]
    async fn watcher_reply_requires_active_membership() {
        let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
            return;
        };

        let workspace_id = choruz_common::new_id();
        let agent_id = choruz_common::new_id();
        let conversation_id = choruz_common::new_id();
        let content = format!("blocked watcher error {}", choruz_common::new_id());

        let (client, connection) = tokio_postgres::connect(&db_url, NoTls)
            .await
            .expect("connect for watcher membership test");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
            .execute(
                "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'agent', 'Watcher Agent', FALSE, NOW(), NOW())",
                &[&agent_id, &workspace_id],
            )
            .await
            .expect("seed watcher agent principal");
        client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, 'direct', NULL, $3, NOW(), NOW())",
                &[&conversation_id, &workspace_id, &agent_id],
            )
            .await
            .expect("seed watcher conversation");

        let store = EventStore::new(&db_url);
        publish_watcher_reply(
            &store,
            "watcher:test",
            &conversation_id,
            &agent_id,
            &content,
        )
        .await;

        let row = client
            .query_one(
                "SELECT COUNT(*)::BIGINT
                 FROM conversation_events
                 WHERE conversation_id = $1 AND content = $2",
                &[&conversation_id, &Some(content.as_str())],
            )
            .await
            .expect("count watcher replies");
        assert_eq!(row.get::<_, i64>(0), 0);
    }
}
