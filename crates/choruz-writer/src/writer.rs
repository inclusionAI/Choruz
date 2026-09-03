//! Conversation Writer core: commits agent reply events to the conversation
//! event stream with turn_id-based deduplication.
//!
//! The writer consumes `AgentResult` values (from the executor or a channel)
//! and for each succeeded result:
//! 1. Checks if the turn_id has already been committed (dedup)
//! 2. Writes a reply event to `conversation_events` with the turn_id
//! 3. The UNIQUE constraint on turn_id prevents duplicate commits from
//!    late-arriving retry attempts.

use choruz_ids::ReplyEventId;
use choruz_store::ConversationEvent;
use tokio::sync::mpsc;
use tracing;

use crate::models::{AgentResult, AgentResultStatus, WriteOutcome};

// ---------------------------------------------------------------------------
// Writer error
// ---------------------------------------------------------------------------

/// Errors from the conversation writer.
#[derive(Debug, thiserror::Error)]
pub enum WriterError {
    #[error("store error: {0}")]
    Store(#[from] choruz_common::AppError),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("stale attempt {attempt_id} for command {command_id}")]
    StaleAttempt {
        command_id: String,
        attempt_id: String,
    },
}

pub type WriterResult<T> = Result<T, WriterError>;

// ---------------------------------------------------------------------------
// ResultStore trait — abstracts DB operations for testing
// ---------------------------------------------------------------------------

/// Abstracts the database operations needed by the writer.
#[allow(async_fn_in_trait)]
pub trait ResultStore {
    /// Check if a turn_id has already been committed to conversation_events.
    async fn turn_already_committed(&self, turn_id: &str) -> WriterResult<bool>;

    /// Insert a reply event into conversation_events.
    /// Returns (event_id, seq) on success, or a Conflict error if the
    /// turn_id unique constraint is violated.
    async fn insert_reply_event(
        &self,
        event: &ConversationEvent,
        command_id: &str,
        attempt_id: &str,
    ) -> WriterResult<(String, i64)>;

    /// Return whether an execution attempt still owns the command.
    async fn attempt_is_current(&self, command_id: &str, attempt_id: &str) -> WriterResult<bool>;

    /// Mark a command as committed and release its session lease.
    async fn mark_committed(&self, command_id: &str, attempt_id: &str) -> WriterResult<()>;
}

// ---------------------------------------------------------------------------
// InMemoryResultStore — for testing
// ---------------------------------------------------------------------------

/// A test-friendly in-memory result store.
#[derive(Default, Clone)]
pub struct InMemoryResultStore {
    pub committed_turns: std::sync::Arc<tokio::sync::Mutex<Vec<String>>>,
    pub events: std::sync::Arc<tokio::sync::Mutex<Vec<ConversationEvent>>>,
    pub committed_commands: std::sync::Arc<tokio::sync::Mutex<Vec<String>>>,
    /// Test-fixture shortcut: an absent command is treated as current.
    /// PostgreSQL remains strict and rejects missing command rows.
    pub current_attempts:
        std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, String>>>,
    #[cfg(test)]
    invalidate_attempt_during_insert: std::sync::Arc<tokio::sync::Mutex<bool>>,
    next_seq: std::sync::Arc<tokio::sync::Mutex<i64>>,
}

fn in_memory_attempt_is_current(
    attempts: &std::collections::HashMap<String, String>,
    command_id: &str,
    attempt_id: &str,
) -> bool {
    attempts
        .get(command_id)
        .is_none_or(|current| current == attempt_id)
}

impl ResultStore for InMemoryResultStore {
    async fn turn_already_committed(&self, turn_id: &str) -> WriterResult<bool> {
        let committed = self.committed_turns.lock().await;
        Ok(committed.contains(&turn_id.to_string()))
    }

    async fn insert_reply_event(
        &self,
        event: &ConversationEvent,
        command_id: &str,
        attempt_id: &str,
    ) -> WriterResult<(String, i64)> {
        #[cfg(test)]
        if *self.invalidate_attempt_during_insert.lock().await {
            self.current_attempts
                .lock()
                .await
                .insert(command_id.to_string(), "superseding-attempt".to_string());
        }
        // Match the PostgreSQL transaction: ownership cannot change between
        // validation and the in-memory persistence writes below.
        let attempts = self.current_attempts.lock().await;
        if !in_memory_attempt_is_current(&attempts, command_id, attempt_id) {
            return Err(WriterError::StaleAttempt {
                command_id: command_id.to_string(),
                attempt_id: attempt_id.to_string(),
            });
        }
        let mut committed = self.committed_turns.lock().await;
        if let Some(ref tid) = event.turn_id {
            if committed.contains(tid) {
                return Err(WriterError::Store(choruz_common::AppError::Conflict(
                    format!("duplicate turn_id: {tid}"),
                )));
            }
            committed.push(tid.clone());
        }
        let mut seq = self.next_seq.lock().await;
        *seq += 1;
        let current_seq = *seq;
        drop(seq);

        self.events.lock().await.push(event.clone());
        Ok((event.event_id.clone(), current_seq))
    }

    async fn attempt_is_current(&self, command_id: &str, attempt_id: &str) -> WriterResult<bool> {
        let attempts = self.current_attempts.lock().await;
        Ok(in_memory_attempt_is_current(
            &attempts, command_id, attempt_id,
        ))
    }

    async fn mark_committed(&self, command_id: &str, attempt_id: &str) -> WriterResult<()> {
        // Keep ownership stable until the command-state write completes.
        let attempts = self.current_attempts.lock().await;
        if !in_memory_attempt_is_current(&attempts, command_id, attempt_id) {
            return Err(WriterError::StaleAttempt {
                command_id: command_id.to_string(),
                attempt_id: attempt_id.to_string(),
            });
        }
        self.committed_commands
            .lock()
            .await
            .push(command_id.to_string());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// commit_result — the core writer function
// ---------------------------------------------------------------------------

/// Attempt to commit an agent result as a reply event.
///
/// Returns a `WriteOutcome` indicating what happened.
pub async fn commit_result<S: ResultStore>(
    result: &AgentResult,
    store: &S,
) -> WriterResult<WriteOutcome> {
    // G1: Only process succeeded results
    if result.status != AgentResultStatus::Succeeded {
        tracing::info!(
            turn_id = %result.turn_id,
            status = ?result.status,
            "skipping non-succeeded result"
        );
        return Ok(WriteOutcome::SkippedNotSucceeded);
    }

    if !store
        .attempt_is_current(&result.command_id, &result.attempt_id)
        .await?
    {
        tracing::info!(
            command_id = %result.command_id,
            attempt_id = %result.attempt_id,
            "skipping stale execution result"
        );
        return Ok(WriteOutcome::SkippedStaleAttempt);
    }

    // G1b: Skip empty content (e.g. group send already handled by outbox_watcher)
    if result.content.as_deref().unwrap_or("").trim().is_empty() {
        tracing::debug!(
            turn_id = %result.turn_id,
            "skipping empty content (group send handled by outbox_watcher)"
        );
        mark_all_committed(result, store).await?;
        return Ok(WriteOutcome::SkippedEmptyCommitted);
    }

    // G2: turn_id dedup check
    if store.turn_already_committed(&result.turn_id).await? {
        tracing::info!(
            turn_id = %result.turn_id,
            "turn already committed, skipping duplicate"
        );
        mark_all_committed(result, store).await?;
        return Ok(WriteOutcome::DuplicateTurn);
    }

    // G3: Write reply event to conversation_events
    let reply_event_id = ReplyEventId::new();

    let event = ConversationEvent {
        conversation_id: result.conversation_id.clone(),
        event_id: reply_event_id.to_string(),
        event_type: "reply".into(),
        sender_id: result.agent_id.clone(),
        content: result.content.clone(),
        content_type: result
            .content_type
            .clone()
            .unwrap_or_else(|| "text/plain".into()),
        metadata: serde_json::json!({
            "command_id": result.command_id,
            "attempt_id": result.attempt_id,
            "tool_calls_count": result.tool_calls_count,
            "execution_duration_ms": result.execution_duration_ms,
            // Carry the originating FE trace id onto the reply event itself
            // so the round-trip FE click → agent reply is searchable by the
            // same correlator in both log streams and row metadata.
            "trace_id": result.trace_id,
        }),
        client_msg_id: None,
        turn_id: Some(result.turn_id.clone()),
        reply_event_id: Some(reply_event_id.to_string()),
    };

    match store
        .insert_reply_event(&event, &result.command_id, &result.attempt_id)
        .await
    {
        Ok((eid, seq)) => {
            tracing::info!(
                event = "writer_reply_committed",
                trace_id = result.trace_id.as_deref().unwrap_or("none"),
                turn_id = %result.turn_id,
                command_id = %result.command_id,
                attempt_id = %result.attempt_id,
                conversation_id = %result.conversation_id,
                agent_id = %result.agent_id,
                reply_event_id = %eid,
                seq,
                duration_ms = result.execution_duration_ms,
                "reply committed to conversation"
            );

            // G4: Mark command lifecycles as committed after the reply event
            // exists. Batched secondary commands close here too.
            mark_all_committed(result, store).await?;

            Ok(WriteOutcome::Committed {
                reply_event_id: eid,
                seq,
            })
        }
        Err(WriterError::Store(choruz_common::AppError::Conflict(_))) => {
            // Duplicate turn commit — safe to ignore (late retry attempt)
            tracing::info!(
                turn_id = %result.turn_id,
                "duplicate turn commit attempted (unique constraint), safely ignored"
            );
            mark_all_committed(result, store).await?;
            Ok(WriteOutcome::DuplicateTurn)
        }
        Err(WriterError::StaleAttempt { .. }) => Ok(WriteOutcome::SkippedStaleAttempt),
        Err(e) => Err(e),
    }
}

async fn mark_all_committed<S: ResultStore>(result: &AgentResult, store: &S) -> WriterResult<()> {
    let mut commands = vec![(result.command_id.as_str(), result.attempt_id.as_str())];
    commands.extend(
        result
            .secondary_command_attempts
            .iter()
            .map(|command| (command.command_id.as_str(), command.attempt_id.as_str())),
    );
    let primary_command_id = result.command_id.as_str();
    for (command_id, attempt_id) in commands {
        match store.mark_committed(command_id, attempt_id).await {
            Ok(()) => {}
            Err(WriterError::StaleAttempt { .. }) if command_id == primary_command_id => {
                // The reply event is already durable. The superseding attempt
                // owns the command lifecycle now, while turn-id dedup keeps a
                // later commit from publishing the same reply twice.
                tracing::warn!(
                    command_id,
                    attempt_id,
                    "writer: primary attempt was superseded while closing command lifecycle"
                );
            }
            Err(WriterError::StaleAttempt { .. }) => {
                tracing::info!(
                    command_id,
                    attempt_id,
                    "writer: skipping commit for superseded command attempt"
                );
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Writer loop — consumes agent results from a channel
// ---------------------------------------------------------------------------

/// Configuration for the writer loop.
#[derive(Debug, Clone)]
pub struct WriterConfig {
    /// Maximum consecutive errors before the loop pauses.
    pub max_consecutive_errors: u32,
    /// Pause duration after max consecutive errors.
    pub error_pause: std::time::Duration,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            max_consecutive_errors: 10,
            error_pause: std::time::Duration::from_secs(5),
        }
    }
}

/// Run the writer loop, consuming agent results from a channel.
///
/// This function blocks until the receiver is closed.
pub async fn run_writer_loop<S: ResultStore>(
    mut rx: mpsc::Receiver<AgentResult>,
    store: S,
    _config: WriterConfig,
) {
    tracing::info!("Conversation Writer loop started");

    while let Some(result) = rx.recv().await {
        match commit_result(&result, &store).await {
            Ok(outcome) => {
                tracing::info!(
                    event = "writer_result_processed",
                    trace_id = result.trace_id.as_deref().unwrap_or("none"),
                    turn_id = %result.turn_id,
                    command_id = %result.command_id,
                    conversation_id = %result.conversation_id,
                    agent_id = %result.agent_id,
                    outcome = ?outcome,
                    "processed agent result"
                );
            }
            Err(e) => {
                tracing::error!(
                    event = "writer_result_failed",
                    trace_id = result.trace_id.as_deref().unwrap_or("none"),
                    turn_id = %result.turn_id,
                    command_id = %result.command_id,
                    conversation_id = %result.conversation_id,
                    agent_id = %result.agent_id,
                    error = %e,
                    "failed to commit agent result"
                );
            }
        }
    }

    tracing::info!("Conversation Writer loop stopped (channel closed)");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_succeeded_result(turn_id: &str) -> AgentResult {
        AgentResult {
            turn_id: turn_id.into(),
            attempt_id: "attempt-1".into(),
            command_id: "cmd-1".into(),
            session_key: "agent-1:conv-1".into(),
            conversation_id: "conv-1".into(),
            agent_id: "agent-1".into(),
            status: AgentResultStatus::Succeeded,
            content: Some("LGTM, merging.".into()),
            content_type: Some("text/plain".into()),
            error: None,
            tool_calls_count: 2,
            execution_duration_ms: 3000,
            secondary_command_attempts: Vec::new(),
            command_results: Vec::new(),
            trace_id: None,
        }
    }

    fn make_failed_result(turn_id: &str) -> AgentResult {
        AgentResult {
            turn_id: turn_id.into(),
            attempt_id: "attempt-1".into(),
            command_id: "cmd-1".into(),
            session_key: "agent-1:conv-1".into(),
            conversation_id: "conv-1".into(),
            agent_id: "agent-1".into(),
            status: AgentResultStatus::Failed,
            content: None,
            content_type: None,
            error: Some("timeout".into()),
            tool_calls_count: 0,
            execution_duration_ms: 60000,
            secondary_command_attempts: Vec::new(),
            command_results: Vec::new(),
            trace_id: None,
        }
    }

    #[tokio::test]
    async fn commit_succeeded_result() {
        let store = InMemoryResultStore::default();
        let result = make_succeeded_result("turn-1");

        let outcome = commit_result(&result, &store).await.unwrap();
        assert!(matches!(outcome, WriteOutcome::Committed { seq: 1, .. }));

        // Verify the event was stored
        let events = store.events.lock().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "reply");
        assert_eq!(events[0].sender_id, "agent-1");
        assert_eq!(events[0].turn_id, Some("turn-1".into()));
        assert_eq!(events[0].content, Some("LGTM, merging.".into()));
    }

    #[tokio::test]
    async fn skip_failed_result() {
        let store = InMemoryResultStore::default();
        let result = make_failed_result("turn-2");

        let outcome = commit_result(&result, &store).await.unwrap();
        assert_eq!(outcome, WriteOutcome::SkippedNotSucceeded);

        // No event should be stored
        let events = store.events.lock().await;
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn empty_succeeded_result_closes_command_without_reply_event() {
        let store = InMemoryResultStore::default();
        let mut result = make_succeeded_result("turn-empty");
        result.content = Some("   ".to_string());

        let outcome = commit_result(&result, &store).await.unwrap();
        assert_eq!(outcome, WriteOutcome::SkippedEmptyCommitted);

        assert!(store.events.lock().await.is_empty());
        assert_eq!(store.committed_commands.lock().await.as_slice(), ["cmd-1"]);
    }

    #[tokio::test]
    async fn succeeded_result_closes_secondary_batch_commands_after_reply_event() {
        let store = InMemoryResultStore::default();
        let mut result = make_succeeded_result("turn-batch");
        result.secondary_command_attempts = vec![
            crate::models::CommandAttemptRef {
                command_id: "cmd-2".into(),
                attempt_id: "attempt-2".into(),
            },
            crate::models::CommandAttemptRef {
                command_id: "cmd-3".into(),
                attempt_id: "attempt-3".into(),
            },
        ];

        let outcome = commit_result(&result, &store).await.unwrap();
        assert!(matches!(outcome, WriteOutcome::Committed { .. }));

        assert_eq!(store.events.lock().await.len(), 1);
        assert_eq!(
            store.committed_commands.lock().await.as_slice(),
            ["cmd-1", "cmd-2", "cmd-3"]
        );
    }

    #[tokio::test]
    async fn stale_secondary_does_not_prevent_other_batch_commands_from_closing() {
        let store = InMemoryResultStore::default();
        store
            .current_attempts
            .lock()
            .await
            .insert("cmd-2".into(), "new-attempt".into());
        let mut result = make_succeeded_result("turn-partially-stale-batch");
        result.secondary_command_attempts = vec![
            crate::models::CommandAttemptRef {
                command_id: "cmd-2".into(),
                attempt_id: "stale-attempt".into(),
            },
            crate::models::CommandAttemptRef {
                command_id: "cmd-3".into(),
                attempt_id: "attempt-3".into(),
            },
        ];

        let outcome = commit_result(&result, &store).await.unwrap();

        assert!(matches!(outcome, WriteOutcome::Committed { .. }));
        assert_eq!(
            store.committed_commands.lock().await.as_slice(),
            ["cmd-1", "cmd-3"]
        );
    }

    #[tokio::test]
    async fn dedup_by_turn_id() {
        let store = InMemoryResultStore::default();

        // First commit succeeds
        let result1 = make_succeeded_result("turn-3");
        let outcome1 = commit_result(&result1, &store).await.unwrap();
        assert!(matches!(outcome1, WriteOutcome::Committed { .. }));

        // Second attempt with same turn_id is deduped (pre-check)
        let result2 = AgentResult {
            attempt_id: "attempt-2".into(),
            ..make_succeeded_result("turn-3")
        };
        let outcome2 = commit_result(&result2, &store).await.unwrap();
        assert_eq!(outcome2, WriteOutcome::DuplicateTurn);

        // Only one event should exist
        let events = store.events.lock().await;
        assert_eq!(events.len(), 1);
        assert_eq!(
            store.committed_commands.lock().await.as_slice(),
            ["cmd-1", "cmd-1"]
        );
    }

    #[tokio::test]
    async fn stale_attempt_cannot_commit_reply_or_command_state() {
        let store = InMemoryResultStore::default();
        store
            .current_attempts
            .lock()
            .await
            .insert("cmd-1".into(), "attempt-2".into());
        let stale = make_succeeded_result("turn-stale");

        let outcome = commit_result(&stale, &store).await.unwrap();

        assert_eq!(outcome, WriteOutcome::SkippedStaleAttempt);
        assert!(store.events.lock().await.is_empty());
        assert!(store.committed_commands.lock().await.is_empty());
    }

    #[tokio::test]
    async fn attempt_superseded_after_precheck_cannot_commit_reply_or_state() {
        let store = InMemoryResultStore::default();
        store
            .current_attempts
            .lock()
            .await
            .insert("cmd-1".into(), "attempt-1".into());
        *store.invalidate_attempt_during_insert.lock().await = true;

        let outcome = commit_result(&make_succeeded_result("turn-racing-stale"), &store)
            .await
            .unwrap();

        assert_eq!(outcome, WriteOutcome::SkippedStaleAttempt);
        assert!(store.events.lock().await.is_empty());
        assert!(store.committed_commands.lock().await.is_empty());
    }

    #[tokio::test]
    async fn different_turn_ids_both_committed() {
        let store = InMemoryResultStore::default();

        let result1 = make_succeeded_result("turn-a");
        let result2 = AgentResult {
            turn_id: "turn-b".into(),
            command_id: "cmd-2".into(),
            ..make_succeeded_result("turn-a")
        };

        let outcome1 = commit_result(&result1, &store).await.unwrap();
        let outcome2 = commit_result(&result2, &store).await.unwrap();

        assert!(matches!(outcome1, WriteOutcome::Committed { seq: 1, .. }));
        assert!(matches!(outcome2, WriteOutcome::Committed { seq: 2, .. }));

        let events = store.events.lock().await;
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn reply_event_has_correct_metadata() {
        let store = InMemoryResultStore::default();
        let result = make_succeeded_result("turn-meta");

        commit_result(&result, &store).await.unwrap();

        let events = store.events.lock().await;
        let meta = &events[0].metadata;
        assert_eq!(meta["command_id"], "cmd-1");
        assert_eq!(meta["attempt_id"], "attempt-1");
        assert_eq!(meta["tool_calls_count"], 2);
        assert_eq!(meta["execution_duration_ms"], 3000);
    }

    #[tokio::test]
    async fn reply_event_has_reply_event_id() {
        let store = InMemoryResultStore::default();
        let result = make_succeeded_result("turn-reid");

        commit_result(&result, &store).await.unwrap();

        let events = store.events.lock().await;
        assert!(events[0].reply_event_id.is_some());
        // reply_event_id should equal event_id
        assert_eq!(events[0].reply_event_id, Some(events[0].event_id.clone()));
    }

    #[tokio::test]
    async fn content_type_defaults_to_text_plain() {
        let store = InMemoryResultStore::default();
        let mut result = make_succeeded_result("turn-ct");
        result.content_type = None;

        commit_result(&result, &store).await.unwrap();

        let events = store.events.lock().await;
        assert_eq!(events[0].content_type, "text/plain");
    }

    #[tokio::test]
    async fn writer_loop_processes_multiple_results() {
        let store = InMemoryResultStore::default();
        let (tx, rx) = mpsc::channel(10);

        // Send several results
        let results = vec![
            make_succeeded_result("turn-loop-1"),
            make_failed_result("turn-loop-2"),
            make_succeeded_result("turn-loop-3"),
        ];

        let store_clone = store.clone();
        let handle = tokio::spawn(async move {
            run_writer_loop(rx, store_clone, WriterConfig::default()).await;
        });

        for r in results {
            tx.send(r).await.unwrap();
        }
        drop(tx); // Close channel to stop loop

        handle.await.unwrap();

        // Only 2 succeeded results should have created events
        let events = store.events.lock().await;
        assert_eq!(events.len(), 2);
    }
}
