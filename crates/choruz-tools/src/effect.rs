//! Effect Journal: domain types and CRUD operations for the `effect_journal` table.
//!
//! The effect journal records every external tool invocation, enabling
//! idempotent replay. Each record is keyed by [`ToolCallId`] and tracks
//! the lifecycle: `pending -> executing -> succeeded | failed`.

use choruz_ids::{AttemptId, ToolCallId, TurnId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// Status of an effect in the journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectStatus {
    /// Record created, execution not yet started.
    Pending,
    /// Execution is in progress.
    Executing,
    /// Execution completed successfully; `tool_output` contains the result.
    Succeeded,
    /// Execution failed; may be retried.
    Failed,
}

impl EffectStatus {
    /// Convert to the database TEXT representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Executing => "executing",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    /// Parse from the database TEXT representation.
    pub fn from_db(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "executing" => Some(Self::Executing),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

impl std::fmt::Display for EffectStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Input for creating a new effect journal entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewEffect {
    pub tool_call_id: ToolCallId,
    pub attempt_id: AttemptId,
    pub turn_id: TurnId,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub is_mutating: bool,
    pub external_idempotency_key: Option<String>,
}

/// A row from the `effect_journal` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectRecord {
    pub tool_call_id: ToolCallId,
    pub attempt_id: AttemptId,
    pub turn_id: TurnId,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub tool_output: Option<serde_json::Value>,
    pub external_idempotency_key: Option<String>,
    pub status: EffectStatus,
    pub is_mutating: bool,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Database CRUD (requires a tokio-postgres client)
// ---------------------------------------------------------------------------

/// Insert a new effect into the journal with `pending` status.
///
/// Returns `Err(Conflict)` if a record with the same `tool_call_id` already
/// exists.
pub async fn insert_effect(
    client: &tokio_postgres::Client,
    effect: &NewEffect,
) -> choruz_common::AppResult<()> {
    let tool_call_id_str = effect.tool_call_id.to_string();
    let attempt_id_str = effect.attempt_id.to_string();
    let turn_id_str = effect.turn_id.to_string();

    client
        .execute(
            "INSERT INTO effect_journal
                (tool_call_id, attempt_id, turn_id, tool_name, tool_input,
                 status, is_mutating, external_idempotency_key, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())",
            &[
                &tool_call_id_str,
                &attempt_id_str,
                &turn_id_str,
                &effect.tool_name,
                &effect.tool_input,
                &EffectStatus::Pending.as_str(),
                &effect.is_mutating,
                &effect.external_idempotency_key,
            ],
        )
        .await
        .map_err(|e| {
            if let Some(db_err) = e.as_db_error()
                && db_err.code() == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION
            {
                return choruz_common::AppError::Conflict(format!(
                    "effect already exists for tool_call_id={}",
                    effect.tool_call_id,
                ));
            }
            choruz_common::AppError::Internal(format!("failed to insert effect: {e}"))
        })?;

    Ok(())
}

/// Retrieve an effect by its [`ToolCallId`].
pub async fn get_effect(
    client: &tokio_postgres::Client,
    tool_call_id: &ToolCallId,
) -> choruz_common::AppResult<Option<EffectRecord>> {
    let id_str = tool_call_id.to_string();
    let row = client
        .query_opt(
            "SELECT tool_call_id, attempt_id, turn_id, tool_name, tool_input,
                    tool_output, external_idempotency_key, status, is_mutating,
                    created_at, completed_at
             FROM effect_journal
             WHERE tool_call_id = $1",
            &[&id_str],
        )
        .await
        .map_err(|e| choruz_common::AppError::Internal(format!("failed to query effect: {e}")))?;

    match row {
        Some(r) => Ok(Some(row_to_effect(&r)?)),
        None => Ok(None),
    }
}

/// Update the status and optional output of an effect.
///
/// Sets `completed_at = NOW()` when the new status is `succeeded` or `failed`.
pub async fn update_effect_result(
    client: &tokio_postgres::Client,
    tool_call_id: &ToolCallId,
    result: Option<&serde_json::Value>,
    status: EffectStatus,
) -> choruz_common::AppResult<()> {
    let id_str = tool_call_id.to_string();
    let status_str = status.as_str();

    let completed_at: Option<DateTime<Utc>> =
        if status == EffectStatus::Succeeded || status == EffectStatus::Failed {
            Some(Utc::now())
        } else {
            None
        };

    let affected = client
        .execute(
            "UPDATE effect_journal
             SET status = $2, tool_output = $3, completed_at = $4
             WHERE tool_call_id = $1",
            &[&id_str, &status_str, &result, &completed_at],
        )
        .await
        .map_err(|e| choruz_common::AppError::Internal(format!("failed to update effect: {e}")))?;

    if affected == 0 {
        return Err(choruz_common::AppError::NotFound(format!(
            "no effect found for tool_call_id={tool_call_id}",
        )));
    }
    Ok(())
}

/// Transition an effect from `pending` to `executing`.
///
/// This uses a conditional UPDATE to prevent races: only one caller
/// succeeds when multiple workers try to claim the same effect.
/// Returns `true` if this caller won the race.
pub async fn claim_effect(
    client: &tokio_postgres::Client,
    tool_call_id: &ToolCallId,
) -> choruz_common::AppResult<bool> {
    let id_str = tool_call_id.to_string();

    let affected = client
        .execute(
            "UPDATE effect_journal
             SET status = 'executing'
             WHERE tool_call_id = $1 AND status = 'pending'",
            &[&id_str],
        )
        .await
        .map_err(|e| choruz_common::AppError::Internal(format!("failed to claim effect: {e}")))?;

    Ok(affected > 0)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn row_to_effect(row: &tokio_postgres::Row) -> choruz_common::AppResult<EffectRecord> {
    let status_str: String = row.get("status");
    let status = EffectStatus::from_db(&status_str).ok_or_else(|| {
        choruz_common::AppError::Internal(format!("unknown effect status: {status_str}"))
    })?;

    let tool_call_id_str: String = row.get("tool_call_id");
    let attempt_id_str: String = row.get("attempt_id");
    let turn_id_str: String = row.get("turn_id");

    Ok(EffectRecord {
        tool_call_id: tool_call_id_str
            .parse()
            .map_err(|e| choruz_common::AppError::Internal(format!("invalid tool_call_id: {e}")))?,
        attempt_id: attempt_id_str
            .parse()
            .map_err(|e| choruz_common::AppError::Internal(format!("invalid attempt_id: {e}")))?,
        turn_id: turn_id_str
            .parse()
            .map_err(|e| choruz_common::AppError::Internal(format!("invalid turn_id: {e}")))?,
        tool_name: row.get("tool_name"),
        tool_input: row.get("tool_input"),
        tool_output: row.get("tool_output"),
        external_idempotency_key: row.get("external_idempotency_key"),
        status,
        is_mutating: row.get("is_mutating"),
        created_at: row.get("created_at"),
        completed_at: row.get("completed_at"),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_status_roundtrip() {
        for status in [
            EffectStatus::Pending,
            EffectStatus::Executing,
            EffectStatus::Succeeded,
            EffectStatus::Failed,
        ] {
            let s = status.as_str();
            let parsed = EffectStatus::from_db(s).unwrap();
            assert_eq!(status, parsed, "roundtrip failed for {s}");
        }
    }

    #[test]
    fn effect_status_from_db_rejects_unknown() {
        assert!(EffectStatus::from_db("running").is_none());
        assert!(EffectStatus::from_db("").is_none());
    }

    #[test]
    fn effect_status_display() {
        assert_eq!(EffectStatus::Pending.to_string(), "pending");
        assert_eq!(EffectStatus::Succeeded.to_string(), "succeeded");
    }

    #[test]
    fn new_effect_serde_roundtrip() {
        let effect = NewEffect {
            tool_call_id: ToolCallId::new(),
            attempt_id: AttemptId::new(),
            turn_id: TurnId::new(),
            tool_name: "github_comment".into(),
            tool_input: serde_json::json!({"body": "LGTM"}),
            is_mutating: true,
            external_idempotency_key: Some("echat:abc-123".into()),
        };

        let json = serde_json::to_string(&effect).expect("serialize");
        let parsed: NewEffect = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.tool_name, "github_comment");
        assert_eq!(parsed.tool_call_id, effect.tool_call_id);
        assert!(parsed.is_mutating);
    }

    #[test]
    fn effect_record_serde_roundtrip() {
        let record = EffectRecord {
            tool_call_id: ToolCallId::new(),
            attempt_id: AttemptId::new(),
            turn_id: TurnId::new(),
            tool_name: "jira_create".into(),
            tool_input: serde_json::json!({"project": "CHORUZ"}),
            tool_output: Some(serde_json::json!({"ticket_id": "CHORUZ-42"})),
            external_idempotency_key: None,
            status: EffectStatus::Succeeded,
            is_mutating: true,
            created_at: Utc::now(),
            completed_at: Some(Utc::now()),
        };

        let json = serde_json::to_string(&record).expect("serialize");
        let parsed: EffectRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.status, EffectStatus::Succeeded);
        assert_eq!(parsed.tool_name, "jira_create");
        assert!(parsed.tool_output.is_some());
    }

    #[test]
    fn effect_status_serde_snake_case() {
        let json = serde_json::to_string(&EffectStatus::Succeeded).unwrap();
        assert_eq!(json, "\"succeeded\"");
        let parsed: EffectStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, EffectStatus::Succeeded);
    }
}
