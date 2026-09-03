//! PostgreSQL-backed EventSource for the Fanout Gateway.
//!
//! Replaces `InMemoryEventSource` with a real implementation that queries
//! `conversation_events` via `EventStore::get_events_after_seq`, plus
//! `conversation_member` for the user-scoped fanout routing.

use choruz_fanout::{EventSource, FanoutError, FanoutResult};
use choruz_store::{ConversationEventRow, EventStore};

/// Provides conversation events from PostgreSQL to the fanout gateway.
#[derive(Clone)]
pub struct PgEventSource {
    store: EventStore,
}

impl PgEventSource {
    pub fn new(store: EventStore) -> Self {
        Self { store }
    }
}

impl EventSource for PgEventSource {
    async fn fetch_events_after(
        &self,
        conversation_id: &str,
        after_seq: i64,
        limit: i64,
    ) -> FanoutResult<Vec<ConversationEventRow>> {
        self.store
            .get_events_after_seq(conversation_id, after_seq, limit)
            .await
            .map_err(FanoutError::Store)
    }

    async fn get_conversation_members(&self, conversation_id: &str) -> FanoutResult<Vec<String>> {
        let client = self
            .store
            .connect()
            .await
            .map_err(|e| FanoutError::EventSource(format!("db connect: {e}")))?;
        let rows = client
            .query(
                "SELECT principal_id FROM conversation_member
                 WHERE conv_id = $1 AND removed_at IS NULL",
                &[&conversation_id],
            )
            .await
            .map_err(|e| FanoutError::EventSource(format!("query members: {e}")))?;
        Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
    }

    async fn get_user_conversations(&self, user_id: &str) -> FanoutResult<Vec<String>> {
        let client = self
            .store
            .connect()
            .await
            .map_err(|e| FanoutError::EventSource(format!("db connect: {e}")))?;
        let rows = client
            .query(
                "SELECT conv_id FROM conversation_member
                 WHERE principal_id = $1 AND removed_at IS NULL",
                &[&user_id],
            )
            .await
            .map_err(|e| FanoutError::EventSource(format!("query user conversations: {e}")))?;
        Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pg_event_source_can_be_constructed() {
        // Verifies the type compiles and can be instantiated.
        let store = EventStore::new(choruz_common::PgConfig::from_env().to_connect_string());
        let _source = PgEventSource::new(store);
    }
}
