//! PostgreSQL-backed ResultStore for the Conversation Writer.
//!
//! Bridges `choruz-writer::ResultStore` to the real `conversation_events` table.

use choruz_session::{PgSessionStore, SessionStatus, SessionUpdate};
use choruz_store::{ConversationEvent, EventStore};
use choruz_writer::{ResultStore, WriterError, WriterResult};

/// Provides turn dedup and event insertion backed by PostgreSQL.
#[derive(Clone)]
pub struct PgResultStore {
    store: EventStore,
    session_store: PgSessionStore,
}

impl PgResultStore {
    pub fn new(store: EventStore, session_store: PgSessionStore) -> Self {
        Self {
            store,
            session_store,
        }
    }
}

impl ResultStore for PgResultStore {
    async fn turn_already_committed(&self, turn_id: &str) -> WriterResult<bool> {
        let client = self.store.connect().await?;

        let row = client
            .query_opt(
                "SELECT 1 FROM conversation_events WHERE turn_id = $1 LIMIT 1",
                &[&turn_id],
            )
            .await
            .map_err(|e| {
                WriterError::Store(choruz_common::AppError::Internal(format!(
                    "turn_already_committed: {e}"
                )))
            })?;

        Ok(row.is_some())
    }

    async fn insert_reply_event(
        &self,
        event: &ConversationEvent,
        command_id: &str,
        attempt_id: &str,
    ) -> WriterResult<(String, i64)> {
        // Reply insert and total_msg_count bump must be one transaction —
        // otherwise a failed UPDATE leaves the message in the store but
        // unread counts permanently drift (visible to every member forever).
        let mut client = self.store.connect().await.map_err(|e| {
            WriterError::Store(choruz_common::AppError::Internal(format!(
                "connect for insert_reply_event: {e}"
            )))
        })?;
        let tx = client.transaction().await.map_err(|e| {
            WriterError::Store(choruz_common::AppError::Internal(format!(
                "begin tx for insert_reply_event: {e}"
            )))
        })?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&event.conversation_id],
        )
        .await
        .map_err(|e| {
            WriterError::Store(choruz_common::AppError::Internal(format!(
                "advisory lock for insert_reply_event: {e}"
            )))
        })?;

        let current_attempt = tx
            .query_opt(
                "SELECT ac.current_attempt_id
                 FROM agent_commands ac
                 JOIN session_registry sr ON sr.session_key = ac.session_key
                 WHERE ac.command_id = $1 AND ac.current_epoch = sr.epoch
                 FOR UPDATE OF ac",
                &[&command_id],
            )
            .await
            .map_err(|e| {
                WriterError::Store(choruz_common::AppError::Internal(format!(
                    "lock command attempt for insert_reply_event: {e}"
                )))
            })?
            .and_then(|row| row.get::<_, Option<String>>("current_attempt_id"));
        if current_attempt.as_deref() != Some(attempt_id) {
            return Err(WriterError::StaleAttempt {
                command_id: command_id.to_string(),
                attempt_id: attempt_id.to_string(),
            });
        }
        let result = self
            .store
            .insert_conversation_event_with_client(&tx, event)
            .await?;

        tx.execute(
            "UPDATE conversation SET total_msg_count = total_msg_count + 1 WHERE id = $1",
            &[&event.conversation_id],
        )
        .await
        .map_err(|e| {
            WriterError::Store(choruz_common::AppError::Internal(format!(
                "bump total_msg_count in insert_reply_event: {e}"
            )))
        })?;

        tx.commit().await.map_err(|e| {
            WriterError::Store(choruz_common::AppError::Internal(format!(
                "commit insert_reply_event: {e}"
            )))
        })?;

        Ok(result)
    }

    async fn attempt_is_current(&self, command_id: &str, attempt_id: &str) -> WriterResult<bool> {
        self.session_store
            .command_attempt_is_current(command_id, attempt_id)
            .await
            .map_err(|e| WriterError::Internal(format!("failed to validate command attempt: {e}")))
    }

    async fn mark_committed(&self, command_id: &str, attempt_id: &str) -> WriterResult<()> {
        // 1. Get command to find session key
        let cmd = self
            .session_store
            .get_command(command_id)
            .await
            .map_err(|e| WriterError::Internal(format!("failed to get command: {e}")))?
            .ok_or_else(|| WriterError::Internal(format!("command not found: {command_id}")))?;
        let session_key = cmd.session_key;

        // 2. Mark command as committed and release lease
        self.session_store
            .mark_command_committed_for_attempt(command_id, attempt_id)
            .await
            .map_err(|e| match e {
                choruz_session::SessionError::StaleAttempt { .. } => WriterError::StaleAttempt {
                    command_id: command_id.to_string(),
                    attempt_id: attempt_id.to_string(),
                },
                other => {
                    WriterError::Internal(format!("failed to mark command committed: {other}"))
                }
            })?;

        // 3. Check for more active commands; if none, transition to draining
        match self
            .session_store
            .find_active_command_for_session(&session_key)
            .await
        {
            Ok(None) => {
                let update = SessionUpdate {
                    session_key,
                    status: Some(SessionStatus::Draining),
                    executor_node_id: None,
                    last_heartbeat_at: None,
                };
                if let Err(e) = self.session_store.update_session(&update).await {
                    tracing::warn!(error = %e, "session draining update failed");
                }
            }
            _ => {}
        }

        Ok(())
    }
}
