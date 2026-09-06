//! PostgreSQL-backed ResultStore for the Conversation Writer.
//!
//! Bridges `choruz-writer::ResultStore` to the real `conversation_events` table.

use choruz_session::PgSessionStore;
use choruz_store::{ConversationEvent, EventStore};
use choruz_writer::{ResultStore, WriterError, WriterResult};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writer_commit_keeps_the_fenced_session_release() {
        let url = std::env::var("CHORUZ_TEST_DATABASE_URL")
            .expect("source infra/host/setup_test_database.sh before database tests");
        let sessions = PgSessionStore::new(&url);
        let writer = PgResultStore::new(EventStore::new(&url), sessions.clone());
        let session = choruz_common::new_id();
        let agent = choruz_common::new_id();
        let conversation = choruz_common::new_id();
        sessions
            .upsert_session(&session, &agent, &conversation)
            .await
            .unwrap();
        let command = sessions
            .insert_command(&choruz_session::InsertCommand {
                command_id: choruz_common::new_id(),
                route_id: choruz_common::new_id(),
                session_key: session.clone(),
                agent_id: agent,
                conversation_id: conversation,
                message_id: choruz_common::new_id(),
                turn_id: choruz_common::new_id(),
                prompt: "Owned writer lifecycle fixture".into(),
                max_attempts: 3,
                metadata: serde_json::json!({}),
            })
            .await
            .unwrap();
        let lease = sessions
            .assign_lease(&command.command_id, "writer-test")
            .await
            .unwrap();
        writer
            .mark_committed(&command.command_id, &lease.attempt_id)
            .await
            .unwrap();
        let final_session = sessions.get_session(&session).await.unwrap().unwrap();
        assert_eq!(
            final_session.status,
            choruz_session::SessionStatus::Idle,
            "the writer must not overwrite the atomic release with draining"
        );
        assert_eq!(final_session.epoch, lease.epoch);
    }
}

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
        // The fenced transaction owns session release. A later draining write
        // could overwrite a new lease acquired after this commit.
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

        Ok(())
    }
}
