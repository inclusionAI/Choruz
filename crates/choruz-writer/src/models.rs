//! Domain models for the Conversation Writer.

use serde::{Deserialize, Serialize};

/// A secondary command represented by the same batched execution result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandAttemptRef {
    pub command_id: String,
    pub attempt_id: String,
}

// ---------------------------------------------------------------------------
// Agent Result (input to the writer)
// ---------------------------------------------------------------------------

/// An agent execution result received from the executor.
///
/// This is the writer's view of the result — it contains all information
/// needed to commit a reply event to the conversation stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResult {
    pub turn_id: String,
    pub attempt_id: String,
    pub command_id: String,
    pub session_key: String,
    pub conversation_id: String,
    pub agent_id: String,
    pub status: AgentResultStatus,
    pub content: Option<String>,
    pub content_type: Option<String>,
    pub error: Option<String>,
    pub tool_calls_count: i32,
    pub execution_duration_ms: i64,
    /// Additional command rows represented by the same execution result.
    ///
    /// Multi-command dispatch batches execute once and commit one reply event;
    /// after that durable commit, the writer closes these secondary command
    /// lifecycles alongside the primary command.
    #[serde(default)]
    pub secondary_command_attempts: Vec<CommandAttemptRef>,
    /// Structured non-chat command results produced by outbox side-effect
    /// commands. Writers must not turn these into timeline replies.
    #[serde(default)]
    pub command_results: Vec<serde_json::Value>,
    /// Originating FE trace id, forwarded end-to-end from the gateway's
    /// `send_message` → router command metadata → executor → writer. Lets
    /// every reply committed to the conversation stream be searched by
    /// the same correlator as the original user action. `None` when the
    /// chain started without an `x-trace-id` header (e.g. server-initiated).
    #[serde(default)]
    pub trace_id: Option<String>,
}

/// Status of the agent result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentResultStatus {
    Succeeded,
    Failed,
}

// ---------------------------------------------------------------------------
// Write Result
// ---------------------------------------------------------------------------

/// Outcome of attempting to commit a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// Reply successfully committed to the conversation stream.
    Committed { reply_event_id: String, seq: i64 },
    /// Skipped because the result status was not "succeeded".
    SkippedNotSucceeded,
    /// Skipped because the content was empty, but command lifecycle was closed.
    SkippedEmptyCommitted,
    /// Skipped because this turn_id was already committed (dedup).
    DuplicateTurn,
    /// Skipped because a newer execution attempt owns the command.
    SkippedStaleAttempt,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_result_serde_roundtrip() {
        let command_results = vec![serde_json::json!({
            "command_type": "task_create",
            "ok": false,
            "error_code": "channel_tasks_disabled",
        })];
        let result = AgentResult {
            turn_id: "turn-1".into(),
            attempt_id: "attempt-1".into(),
            command_id: "cmd-1".into(),
            session_key: "agent-1:conv-1".into(),
            conversation_id: "conv-1".into(),
            agent_id: "agent-1".into(),
            status: AgentResultStatus::Succeeded,
            content: Some("LGTM".into()),
            content_type: Some("text".into()),
            error: None,
            tool_calls_count: 3,
            execution_duration_ms: 4500,
            secondary_command_attempts: vec![CommandAttemptRef {
                command_id: "cmd-2".into(),
                attempt_id: "attempt-2".into(),
            }],
            command_results: command_results.clone(),
            trace_id: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: AgentResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.turn_id, result.turn_id);
        assert_eq!(parsed.status, result.status);
        assert_eq!(
            parsed.secondary_command_attempts,
            result.secondary_command_attempts
        );
        assert_eq!(parsed.command_results, command_results);
    }

    #[test]
    fn agent_result_missing_command_results_deserializes_to_empty_vec() {
        let json = serde_json::json!({
            "turn_id": "turn-1",
            "attempt_id": "attempt-1",
            "command_id": "cmd-1",
            "session_key": "agent-1:conv-1",
            "conversation_id": "conv-1",
            "agent_id": "agent-1",
            "status": "succeeded",
            "content": "LGTM",
            "content_type": "text",
            "error": null,
            "tool_calls_count": 3,
            "execution_duration_ms": 4500,
            "secondary_command_attempts": [],
            "trace_id": null,
        });
        let parsed: AgentResult = serde_json::from_value(json).unwrap();
        assert!(parsed.command_results.is_empty());
    }

    #[test]
    fn agent_result_failed_with_error() {
        let result = AgentResult {
            turn_id: "turn-2".into(),
            attempt_id: "attempt-2".into(),
            command_id: "cmd-2".into(),
            session_key: "agent-1:conv-1".into(),
            conversation_id: "conv-1".into(),
            agent_id: "agent-1".into(),
            status: AgentResultStatus::Failed,
            content: None,
            content_type: None,
            error: Some("process crashed".into()),
            tool_calls_count: 0,
            execution_duration_ms: 500,
            secondary_command_attempts: Vec::new(),
            command_results: Vec::new(),
            trace_id: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: AgentResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.status, AgentResultStatus::Failed);
        assert!(parsed.content.is_none());
        assert_eq!(parsed.error, Some("process crashed".into()));
    }

    #[test]
    fn write_outcome_variants() {
        let committed = WriteOutcome::Committed {
            reply_event_id: "re-1".into(),
            seq: 42,
        };
        assert!(matches!(committed, WriteOutcome::Committed { .. }));

        let skipped = WriteOutcome::SkippedNotSucceeded;
        assert_eq!(skipped, WriteOutcome::SkippedNotSucceeded);

        let empty = WriteOutcome::SkippedEmptyCommitted;
        assert_eq!(empty, WriteOutcome::SkippedEmptyCommitted);

        let dup = WriteOutcome::DuplicateTurn;
        assert_eq!(dup, WriteOutcome::DuplicateTurn);
    }

    #[test]
    fn agent_result_status_serde() {
        assert_eq!(
            serde_json::to_string(&AgentResultStatus::Succeeded).unwrap(),
            "\"succeeded\""
        );
        assert_eq!(
            serde_json::to_string(&AgentResultStatus::Failed).unwrap(),
            "\"failed\""
        );
    }
}
