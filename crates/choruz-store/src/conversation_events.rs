//! CRUD operations for the `conversation_events` table.
//!
//! The conversation_events table is an append-only log keyed by
//! `(conversation_id, seq)`.  Each row represents a single event in a
//! conversation (message, reply, reaction, edit, system, etc.).

use choruz_common::{AppError, AppResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::EventStore;

/// Input struct for inserting a new conversation event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationEvent {
    pub conversation_id: String,
    pub event_id: String,
    pub event_type: String,
    pub sender_id: String,
    pub content: Option<String>,
    pub content_type: String,
    pub metadata: serde_json::Value,
    /// Client-generated message ID for retry dedup (user messages).
    pub client_msg_id: Option<String>,
    /// Logical reply turn ID (agent replies).
    pub turn_id: Option<String>,
    /// Reply event ID alias (agent replies).
    pub reply_event_id: Option<String>,
}

/// SQL predicate that tests whether a conversation_events row is a
/// THREADED reply (as opposed to a legacy quote-reply, which has
/// reply_event_id but no thread flag).
///
/// Uses jsonb equality (`metadata->'thread' = 'true'::jsonb`), NOT a
/// `::boolean` cast: a cast throws 22P02 on client-planted non-boolean
/// values (permanent 500 for that thread) and silently accepts the
/// STRING "true", which the Rust write paths (`as_bool()`) treat as
/// false — mis-rooting replies. jsonb equality matches exactly the JSON
/// boolean `true`, mirroring `as_bool()`.
///
/// Every SQL site that discriminates threaded replies MUST interpolate
/// this constant (via `format!`) instead of hand-copying the predicate,
/// so the write-path/read-path semantics cannot drift. When the events
/// table carries an alias, build the predicate with
/// [`thread_flag_sql_for`] instead of string-replacing this constant.
pub const THREAD_FLAG_SQL: &str = "COALESCE(metadata->'thread' = 'true'::jsonb, false)";

/// Build the [`THREAD_FLAG_SQL`] predicate for an aliased
/// conversation_events table (e.g. `thread_flag_sql_for("ce")`). This is
/// the ONLY sanctioned way to qualify the predicate — textual
/// `.replace()` on the constant would silently corrupt queries if the
/// constant ever gains another occurrence of the replaced token.
pub fn thread_flag_sql_for(table_alias: &str) -> String {
    format!("COALESCE({table_alias}.metadata->'thread' = 'true'::jsonb, false)")
}

/// Thread discriminator flags parsed from a message's metadata — the
/// single Rust-side source of truth shared by every write path
/// (`DbService::send_message`, the pipeline outbox) and the router's
/// envelope probe, mirroring what [`THREAD_FLAG_SQL`] is for SQL.
///
/// Semantics:
/// - `thread: true` (JSON boolean, matching `as_bool()`) marks a THREADED
///   reply; anything else (absent, string "true", etc.) is a legacy
///   quote-reply or plain message.
/// - `broadcast: true` additionally surfaces the reply on the main
///   timeline ("also send to channel").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadFlags {
    pub is_thread_reply: bool,
    pub is_broadcast: bool,
}

impl ThreadFlags {
    /// Parse the discriminator flags from message metadata.
    pub fn from_metadata(metadata: &serde_json::Value) -> Self {
        let flag = |key: &str| metadata.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
        Self {
            is_thread_reply: flag("thread"),
            is_broadcast: flag("broadcast"),
        }
    }

    /// Whether this message bumps the CONVERSATION-level unread counter
    /// (`total_msg_count`). Quiet thread replies don't — they are counted
    /// per thread via `thread_read_receipt` instead. Mentions always bump
    /// regardless (enforced at the mention-count sites, not here).
    pub fn bumps_conversation_unread(&self) -> bool {
        !self.is_thread_reply || self.is_broadcast
    }
}

/// Row returned from the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationEventRow {
    pub conversation_id: String,
    pub seq: i64,
    pub event_id: String,
    pub event_type: String,
    pub sender_id: String,
    pub content: Option<String>,
    pub content_type: String,
    pub metadata: serde_json::Value,
    pub client_msg_id: Option<String>,
    pub turn_id: Option<String>,
    pub reply_event_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl EventStore {
    /// Insert a new conversation event.
    ///
    /// The `seq` is allocated atomically inside the database using
    /// `COALESCE(MAX(seq), 0) + 1` guarded by a per-conversation Postgres
    /// advisory lock to guarantee per-conversation monotonicity even under
    /// concurrent writers (see Bug M).
    ///
    /// Returns the allocated `(event_id, seq)`.
    pub async fn insert_conversation_event(
        &self,
        event: &ConversationEvent,
    ) -> AppResult<(String, i64)> {
        let mut client = self.connect().await?;
        let tx = client.transaction().await.map_err(|e| {
            AppError::Internal(format!("begin tx for insert_conversation_event: {e}"))
        })?;

        // Serialize concurrent writers targeting the same conversation so the
        // COALESCE(MAX(seq), 0) + 1 allocation below cannot race to the same
        // value and collide on the (conversation_id, seq) unique constraint.
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&event.conversation_id],
        )
        .await
        .map_err(|e| {
            AppError::Internal(format!("advisory lock for insert_conversation_event: {e}"))
        })?;

        let result = Self::insert_conversation_event_stmt(&tx, event).await?;

        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("commit insert_conversation_event: {e}")))?;

        Ok(result)
    }

    /// Insert a conversation event using an existing database client or
    /// transaction handle. Accepts any `GenericClient`, so callers that
    /// need to bundle the insert with other statements in the same
    /// transaction can pass their own `Transaction<'_>`.
    ///
    /// IMPORTANT: the caller is responsible for wrapping the call in a
    /// transaction *and* acquiring `pg_advisory_xact_lock(hashtext(conv_id))`
    /// before calling this, otherwise concurrent writers may race on `seq`.
    pub async fn insert_conversation_event_with_client<C>(
        &self,
        client: &C,
        event: &ConversationEvent,
    ) -> AppResult<(String, i64)>
    where
        C: deadpool_postgres::GenericClient,
    {
        Self::insert_conversation_event_stmt(client, event).await
    }

    /// Shared INSERT statement used by both the owned-tx and
    /// caller-provided-client variants above.
    async fn insert_conversation_event_stmt<C>(
        client: &C,
        event: &ConversationEvent,
    ) -> AppResult<(String, i64)>
    where
        C: deadpool_postgres::GenericClient,
    {
        let row = client
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
                 RETURNING event_id, seq",
                &[
                    &event.conversation_id,
                    &event.event_id,
                    &event.event_type,
                    &event.sender_id,
                    &event.content,
                    &event.content_type,
                    &event.metadata,
                    &event.client_msg_id,
                    &event.turn_id,
                    &event.reply_event_id,
                ],
            )
            .await
            .map_err(|e| {
                // Check for unique constraint violations (client_msg_id / turn_id dedup).
                if let Some(db_err) = e.as_db_error()
                    && db_err.code() == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION
                {
                    return AppError::Conflict(format!(
                        "duplicate conversation event: {}",
                        db_err.detail().unwrap_or("unique constraint violated")
                    ));
                }
                AppError::Internal(format!("failed to insert conversation event: {e}"))
            })?;

        let event_id: String = row.get(0);
        let seq: i64 = row.get(1);
        Ok((event_id, seq))
    }

    /// Fetch conversation events after a given sequence number.
    ///
    /// Returns events with `seq > after_seq`, ordered by `seq ASC`,
    /// limited to `limit` rows.
    pub async fn get_events_after_seq(
        &self,
        conversation_id: &str,
        after_seq: i64,
        limit: i64,
    ) -> AppResult<Vec<ConversationEventRow>> {
        let client = self.connect().await?;
        let rows = client
            .query(
                "SELECT conversation_id, seq, event_id, event_type, sender_id,
                        content, content_type, metadata, client_msg_id, turn_id,
                        reply_event_id, created_at
                 FROM conversation_events
                 WHERE conversation_id = $1 AND seq > $2
                 ORDER BY seq ASC
                 LIMIT $3",
                &[&conversation_id, &after_seq, &limit],
            )
            .await
            .map_err(|e| AppError::Internal(format!("failed to query conversation events: {e}")))?;

        Ok(rows.into_iter().map(row_to_event).collect())
    }

    /// Look up a conversation event by its `event_id` (message_id).
    pub async fn get_event_by_message_id(
        &self,
        event_id: &str,
    ) -> AppResult<Option<ConversationEventRow>> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT conversation_id, seq, event_id, event_type, sender_id,
                        content, content_type, metadata, client_msg_id, turn_id,
                        reply_event_id, created_at
                 FROM conversation_events
                 WHERE event_id = $1",
                &[&event_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("failed to query event by message_id: {e}")))?;

        Ok(row.map(row_to_event))
    }

    /// List conversation events that represent messages (for the
    /// `list_messages` DB-fallback path).
    ///
    /// Without a cursor, returns the most recent events newest-first. With
    /// `since_seq`, returns the oldest unseen events first so callers can
    /// advance repeatedly without skipping a burst larger than `limit`.
    pub async fn list_messages_by_conversation(
        &self,
        conversation_id: &str,
        limit: i64,
        since_seq: Option<i64>,
    ) -> AppResult<Vec<ConversationEventRow>> {
        let client = self.connect().await?;
        let rows = if let Some(since) = since_seq {
            client
                .query(
                    "SELECT conversation_id, seq, event_id, event_type, sender_id,
                            content, content_type, metadata, client_msg_id, turn_id,
                            reply_event_id, created_at
                     FROM conversation_events
                     WHERE conversation_id = $1
                       AND event_type IN ('message', 'message.created', 'reply')
                       AND seq > $3
                     ORDER BY seq ASC
                     LIMIT $2",
                    &[&conversation_id, &limit, &since],
                )
                .await
        } else {
            client
                .query(
                    "SELECT conversation_id, seq, event_id, event_type, sender_id,
                            content, content_type, metadata, client_msg_id, turn_id,
                            reply_event_id, created_at
                     FROM conversation_events
                     WHERE conversation_id = $1
                       AND event_type IN ('message', 'message.created', 'reply')
                     ORDER BY seq DESC
                     LIMIT $2",
                    &[&conversation_id, &limit],
                )
                .await
        }
        .map_err(|e| AppError::Internal(format!("list_messages_by_conversation: {e}")))?;

        Ok(rows.into_iter().map(row_to_event).collect())
    }

    /// Look up a conversation event by its `client_msg_id` for dedup.
    pub async fn find_event_by_client_msg_id(
        &self,
        client_msg_id: &str,
    ) -> AppResult<Option<ConversationEventRow>> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                "SELECT conversation_id, seq, event_id, event_type, sender_id,
                        content, content_type, metadata, client_msg_id, turn_id,
                        reply_event_id, created_at
                 FROM conversation_events
                 WHERE client_msg_id = $1",
                &[&client_msg_id],
            )
            .await
            .map_err(|e| {
                AppError::Internal(format!("failed to query event by client_msg_id: {e}"))
            })?;

        Ok(row.map(row_to_event))
    }
}

fn row_to_event(row: tokio_postgres::Row) -> ConversationEventRow {
    ConversationEventRow {
        conversation_id: row.get("conversation_id"),
        seq: row.get("seq"),
        event_id: row.get("event_id"),
        event_type: row.get("event_type"),
        sender_id: row.get("sender_id"),
        content: row.get("content"),
        content_type: row.get("content_type"),
        metadata: row.get("metadata"),
        client_msg_id: row.get("client_msg_id"),
        turn_id: row.get("turn_id"),
        reply_event_id: row.get("reply_event_id"),
        created_at: row.get("created_at"),
    }
}

impl EventStore {
    /// Resolve the canonical thread root for a threaded reply. Shared by both the
    /// write paths (DbService::send_message and the agent outbox's
    /// send_to_group) and the read-side canonicalize-on-read callers
    /// (list_thread_replies, mark_thread_viewed) so the scoping rules and
    /// the thread-discriminator semantics live in exactly one place.
    ///
    /// Takes any `GenericClient`: WRITE paths must call it inside their
    /// per-conversation advisory-lock transaction (the resolved root is
    /// about to be persisted and must not race a concurrent re-root);
    /// READ paths may call it on a plain pooled connection — a stale
    /// answer there is indistinguishable from the read racing the write,
    /// which is inherent. Returns `Ok(None)` when the target does not
    /// exist in this conversation (callers map that to their own error
    /// shape; the uniform not-found keeps the lookup from acting as a
    /// cross-conversation existence oracle).
    ///
    /// Scoping rules (each load-bearing):
    /// - `conversation_id = $2` — cross-conversation targets are
    ///   indistinguishable from missing ones.
    /// - `event_type IN ('message','message.created','reply')` — the
    ///   same message-like set the read paths list; threads cannot
    ///   root on workflow/system/task events.
    /// - The thread discriminator uses jsonb equality
    ///   (`metadata->'thread' = 'true'::jsonb`), NOT a `::boolean`
    ///   cast: a cast throws 22P02 on client-planted non-boolean
    ///   values (permanent 500 for that thread) and silently accepts
    ///   the STRING "true", which the Rust write path (`as_bool()`)
    ///   treats as false — mis-rooting replies. jsonb equality matches
    ///   exactly the JSON boolean `true`.
    pub async fn canonicalize_thread_root_in_tx(
        client: &impl tokio_postgres::GenericClient,
        conversation_id: &str,
        target_id: &str,
    ) -> AppResult<Option<String>> {
        let target_row = client
            .query_opt(
                &format!(
                    "SELECT reply_event_id, {THREAD_FLAG_SQL} AS is_thread \
                     FROM conversation_events \
                     WHERE event_id = $1 \
                       AND conversation_id = $2 \
                       AND event_type IN ('message', 'message.created', 'reply')"
                ),
                &[&target_id, &conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("thread target lookup: {e}")))?;
        let Some(row) = target_row else {
            return Ok(None);
        };
        let target_is_thread_reply: bool = row.get("is_thread");
        let target_parent: Option<String> = row.get("reply_event_id");
        let root = match (target_is_thread_reply, target_parent) {
            // Replying to a threaded reply → canonicalize to its root.
            (true, Some(root)) => root,
            // Replying to a root message, or to a legacy quote-reply —
            // which acts as the root of the new thread growing under
            // it. (true, None) is unreachable for rows the write paths
            // produce, but a hand-crafted row degrades safely to
            // acting as a root.
            _ => target_id.to_string(),
        };
        Ok(Some(root))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_event_serde_roundtrip() {
        let event = ConversationEvent {
            conversation_id: "conv-1".into(),
            event_id: "evt-1".into(),
            event_type: "message".into(),
            sender_id: "user-1".into(),
            content: Some("hello world".into()),
            content_type: "text/plain".into(),
            metadata: serde_json::json!({}),
            client_msg_id: Some("client-1".into()),
            turn_id: None,
            reply_event_id: None,
        };

        let json = serde_json::to_string(&event).expect("serialize");
        let parsed: ConversationEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.conversation_id, "conv-1");
        assert_eq!(parsed.event_id, "evt-1");
        assert_eq!(parsed.content, Some("hello world".into()));
    }

    #[test]
    fn conversation_event_row_serde_roundtrip() {
        let row = ConversationEventRow {
            conversation_id: "conv-1".into(),
            seq: 42,
            event_id: "evt-1".into(),
            event_type: "message".into(),
            sender_id: "user-1".into(),
            content: Some("hello".into()),
            content_type: "text/plain".into(),
            metadata: serde_json::json!({"key": "value"}),
            client_msg_id: None,
            turn_id: None,
            reply_event_id: None,
            created_at: chrono::Utc::now(),
        };

        let json = serde_json::to_string(&row).expect("serialize");
        let parsed: ConversationEventRow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.seq, 42);
        assert_eq!(parsed.event_type, "message");
    }

    #[test]
    fn thread_flags_require_json_booleans() {
        // JSON boolean true → flags set.
        let flags =
            ThreadFlags::from_metadata(&serde_json::json!({"thread": true, "broadcast": true}));
        assert!(flags.is_thread_reply);
        assert!(flags.is_broadcast);
        assert!(flags.bumps_conversation_unread());

        // Quiet thread reply does NOT bump the conversation counter.
        let quiet = ThreadFlags::from_metadata(&serde_json::json!({"thread": true}));
        assert!(quiet.is_thread_reply);
        assert!(!quiet.is_broadcast);
        assert!(!quiet.bumps_conversation_unread());

        // String "true" is NOT a thread — mirrors THREAD_FLAG_SQL's jsonb
        // equality and as_bool() on the write paths.
        let stringly =
            ThreadFlags::from_metadata(&serde_json::json!({"thread": "true", "broadcast": "true"}));
        assert!(!stringly.is_thread_reply);
        assert!(!stringly.is_broadcast);
        assert!(stringly.bumps_conversation_unread());

        // Plain messages and legacy quote-replies bump as before.
        let plain = ThreadFlags::from_metadata(&serde_json::json!({"reply_to_id": "m-1"}));
        assert!(!plain.is_thread_reply);
        assert!(plain.bumps_conversation_unread());
    }

    #[test]
    fn v018_partial_index_predicate_matches_thread_flag_sql() {
        // The V018 partial index's WHERE clause must stay TEXTUALLY
        // identical to THREAD_FLAG_SQL: every thread read path
        // interpolates the constant, and the planner only matches the
        // partial index when the query predicate implies the index
        // predicate. An edit to either side without the other silently
        // demotes those queries to full scans (correct results, dead
        // index) — this tripwire turns that into a test failure.
        let migration = include_str!("../../../migrations/V018__message_threads.sql");
        assert!(
            migration.contains(THREAD_FLAG_SQL),
            "V018 index predicate drifted from THREAD_FLAG_SQL — update them together",
        );
    }

    #[test]
    fn thread_flag_sql_for_qualifies_the_shared_predicate() {
        // The builder must produce exactly THREAD_FLAG_SQL with the alias
        // prefix — guards against the constant and the builder drifting.
        assert_eq!(
            thread_flag_sql_for("ce"),
            THREAD_FLAG_SQL.replacen("metadata", "ce.metadata", 1),
        );
    }
}
