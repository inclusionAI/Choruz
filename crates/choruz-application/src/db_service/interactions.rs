use choruz_common::AppError;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

use super::DbService;

#[derive(Debug, Serialize)]
pub struct InteractionRecord {
    pub event_id: String,
    pub sequence: i64,
    pub sender_id: String,
    pub sender_role: Option<String>,
    pub event_type: String,
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub trace_id: Option<String>,
    pub turn_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub executions: Value,
}

#[derive(Debug, Serialize)]
pub struct InteractionPage {
    pub records: Vec<InteractionRecord>,
    pub next_before_seq: Option<i64>,
}

impl DbService {
    /// Read committed records newest-first in an authorized conversation.
    /// The gateway must apply the shared conversation read gate before calling.
    /// Content is opt-in; prompts, errors and tool payloads are never projected.
    pub async fn list_interactions(
        &self,
        conversation_id: &str,
        before_seq: Option<i64>,
        limit: i64,
        include_content: bool,
    ) -> Result<InteractionPage, AppError> {
        if !(1..=100).contains(&limit) || before_seq.is_some_and(|value| value < 0) {
            return Err(AppError::Validation(
                "interaction limit must be 1–100 and before_seq nonnegative".into(),
            ));
        }
        let client = self.store.connect().await?;
        let rows = client
            .query(
                "SELECT ce.event_id, ce.seq, ce.sender_id, p.type AS sender_role,
                    ce.event_type, ce.content_type, CASE WHEN $4 THEN ce.content END AS content,
                    ce.metadata->>'trace_id' AS trace_id, ce.turn_id, ce.created_at,
                    COALESCE((SELECT jsonb_agg(jsonb_build_object(
                        'command_id', ac.command_id, 'agent_id', ac.agent_id,
                        'turn_id', ac.turn_id, 'status', ac.status,
                        'attempt_count', ac.attempt_count, 'created_at', ac.created_at,
                        'updated_at', ac.updated_at,
                        'attempts', COALESCE((SELECT jsonb_agg(jsonb_build_object(
                            'attempt_id', ar.attempt_id, 'status', ar.status,
                            'tool_calls_count', ar.tool_calls_count,
                            'execution_duration_ms', ar.execution_duration_ms,
                            'created_at', ar.created_at) ORDER BY ar.created_at, ar.id)
                            FROM agent_results ar WHERE ar.command_id = ac.command_id
                              AND ar.conversation_id = ce.conversation_id), '[]'::jsonb)
                        ) ORDER BY ac.created_at, ac.command_id)
                        FROM agent_commands ac WHERE ac.conversation_id = ce.conversation_id
                          AND ac.message_id = ce.event_id), '[]'::jsonb) AS executions
             FROM conversation_events ce LEFT JOIN principal p ON p.id = ce.sender_id
             WHERE ce.conversation_id = $1 AND ($2::bigint IS NULL OR ce.seq < $2)
               AND ce.event_type IN ('message', 'message.created', 'reply')
             ORDER BY ce.seq DESC LIMIT $3",
                &[
                    &conversation_id,
                    &before_seq,
                    &(limit + 1),
                    &include_content,
                ],
            )
            .await
            .map_err(|error| AppError::Internal(format!("list interactions: {error}")))?;
        let has_more = rows.len() > limit as usize;
        let records: Vec<_> = rows
            .into_iter()
            .take(limit as usize)
            .map(|row| InteractionRecord {
                event_id: row.get("event_id"),
                sequence: row.get("seq"),
                sender_id: row.get("sender_id"),
                sender_role: row.get("sender_role"),
                event_type: row.get("event_type"),
                content_type: row.get("content_type"),
                content: row.get("content"),
                trace_id: row.get("trace_id"),
                turn_id: row.get("turn_id"),
                created_at: row.get("created_at"),
                executions: row.get("executions"),
            })
            .collect();
        let next_before_seq = has_more
            .then(|| records.last().map(|row| row.sequence))
            .flatten();
        Ok(InteractionPage {
            records,
            next_before_seq,
        })
    }
}
