//! CRUD operations for the `event_outbox` table.
//!
//! The outbox pattern: events are written to the outbox in the same
//! transaction as the conversation_events row.  A CDC poller then
//! reads unpublished rows, publishes them, and marks them done.

use choruz_common::{AppError, AppResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::EventStore;

/// Input struct for inserting a new outbox entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxEntry {
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
}

/// Row returned from the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxRow {
    pub id: i64,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub published: bool,
    pub claimed_by: Option<String>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub claim_deadline: Option<DateTime<Utc>>,
    pub attempt_count: i32,
}

impl EventStore {
    /// Insert a new outbox entry.
    ///
    /// Returns the auto-generated `id`.
    pub async fn insert_outbox_entry(&self, entry: &OutboxEntry) -> AppResult<i64> {
        let client = self.connect().await?;
        self.insert_outbox_entry_with_client(&client, entry).await
    }

    /// Insert an outbox entry using an existing database client (for
    /// transactional use with `insert_conversation_event`).
    pub async fn insert_outbox_entry_with_client(
        &self,
        client: &deadpool_postgres::Client,
        entry: &OutboxEntry,
    ) -> AppResult<i64> {
        let row = client
            .query_one(
                "INSERT INTO event_outbox
                    (aggregate_type, aggregate_id, event_type, payload, created_at, published)
                 VALUES ($1, $2, $3, $4, NOW(), FALSE)
                 RETURNING id",
                &[
                    &entry.aggregate_type,
                    &entry.aggregate_id,
                    &entry.event_type,
                    &entry.payload,
                ],
            )
            .await
            .map_err(|e| AppError::Internal(format!("failed to insert outbox entry: {e}")))?;

        Ok(row.get(0))
    }

    /// Claim up to `limit` unpublished outbox entries.
    ///
    /// Uses an atomic UPDATE with a lease to allow concurrent pollers without
    /// double-processing. Returns the claimed rows.
    pub async fn claim_unpublished_entries(
        &self,
        limit: i64,
        node_id: &str,
        lease_duration: chrono::Duration,
    ) -> AppResult<Vec<OutboxRow>> {
        let client = self.connect().await?;
        let now = Utc::now();
        let deadline = now + lease_duration;

        // Atomic claim using subquery for LIMIT support in UPDATE
        let rows = client
            .query(
                "UPDATE event_outbox
                 SET claimed_by = $1,
                     claimed_at = $2,
                     claim_deadline = $3,
                     attempt_count = attempt_count + 1
                 WHERE id IN (
                     SELECT id FROM event_outbox
                     WHERE published = FALSE
                       AND (claim_deadline IS NULL OR claim_deadline < $2)
                     ORDER BY id ASC
                     LIMIT $4
                     FOR UPDATE SKIP LOCKED
                 )
                 RETURNING id, aggregate_type, aggregate_id, event_type, payload,
                           created_at, published, claimed_by, claimed_at,
                           claim_deadline, attempt_count",
                &[&node_id, &now, &deadline, &limit],
            )
            .await
            .map_err(|e| AppError::Internal(format!("failed to claim outbox entries: {e}")))?;

        Ok(rows.into_iter().map(row_to_outbox).collect())
    }

    /// Mark one or more outbox entries as published.
    /// Only marks if the claimant matches to ensure lease integrity.
    pub async fn mark_published(&self, ids: &[i64], node_id: &str) -> AppResult<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let client = self.connect().await?;

        let count = client
            .execute(
                "UPDATE event_outbox
                 SET published = TRUE
                 WHERE id = ANY($1) AND claimed_by = $2",
                &[&ids, &node_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("failed to mark outbox published: {e}")))?;

        Ok(count)
    }

    /// Get a single outbox entry by ID (mainly for testing).
    pub async fn get_outbox_entry(&self, id: i64) -> AppResult<Option<OutboxRow>> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT id, aggregate_type, aggregate_id, event_type, payload,
                        created_at, published, claimed_by, claimed_at,
                        claim_deadline, attempt_count
                 FROM event_outbox
                 WHERE id = $1",
                &[&id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("failed to get outbox entry: {e}")))?;

        Ok(row.map(row_to_outbox))
    }
}

fn row_to_outbox(row: tokio_postgres::Row) -> OutboxRow {
    OutboxRow {
        id: row.get("id"),
        aggregate_type: row.get("aggregate_type"),
        aggregate_id: row.get("aggregate_id"),
        event_type: row.get("event_type"),
        payload: row.get("payload"),
        created_at: row.get("created_at"),
        published: row.get("published"),
        claimed_by: row.get("claimed_by"),
        claimed_at: row.get("claimed_at"),
        claim_deadline: row.get("claim_deadline"),
        attempt_count: row.get("attempt_count"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbox_entry_serde_roundtrip() {
        let entry = OutboxEntry {
            aggregate_type: "conversation_event".into(),
            aggregate_id: "conv-1".into(),
            event_type: "message".into(),
            payload: serde_json::json!({
                "message_id": "msg-1",
                "content": "hello"
            }),
        };

        let json = serde_json::to_string(&entry).expect("serialize");
        let parsed: OutboxEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.aggregate_type, "conversation_event");
        assert_eq!(parsed.event_type, "message");
    }

    #[test]
    fn outbox_row_serde_roundtrip() {
        let row = OutboxRow {
            id: 42,
            aggregate_type: "conversation_event".into(),
            aggregate_id: "conv-1".into(),
            event_type: "message".into(),
            payload: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            published: false,
            claimed_by: None,
            claimed_at: None,
            claim_deadline: None,
            attempt_count: 0,
        };

        let json = serde_json::to_string(&row).expect("serialize");
        let parsed: OutboxRow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.id, 42);
        assert!(!parsed.published);
    }
}
