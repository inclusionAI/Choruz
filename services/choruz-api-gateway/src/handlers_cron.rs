//! CRUD handlers for agent cron jobs.

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::HeaderMap,
};
use choruz_common::AppError;
use choruz_domain::PrincipalType;
use serde::{Deserialize, Serialize};

use crate::{ApiError, ApiState, require_human_operator};

async fn require_agent_workspace_access(
    headers: &HeaderMap,
    state: &ApiState,
    agent_id: &str,
) -> Result<String, ApiError> {
    let operator = require_human_operator(headers, state).await?;
    let agent = state.db.get_principal(agent_id).await?;
    if !matches!(agent.principal_type, PrincipalType::Agent) {
        return Err(ApiError::from(AppError::NotFound("agent not found".into())));
    }
    let can_access = operator.workspace_id == agent.workspace_id
        || state
            .db
            .list_companies(&operator.id)
            .await?
            .iter()
            .any(|company| company.id == agent.workspace_id);
    if !can_access {
        return Err(ApiError::from(AppError::Forbidden(
            "cross-workspace access denied".into(),
        )));
    }
    Ok(agent.workspace_id)
}

// ── Response / request types ────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CronJobResponse {
    id: String,
    agent_id: String,
    conversation_id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    schedule_type: String,
    schedule_value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    schedule_timezone: Option<String>,
    message: String,
    session_target: String,
    delivery_mode: String,
    enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_run_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_run_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    consecutive_errors: i32,
    timeout_seconds: i32,
    delete_after_run: bool,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateCronJobRequest {
    pub name: String,
    pub schedule_type: String,
    pub schedule_value: String,
    #[serde(default)]
    pub schedule_timezone: Option<String>,
    pub message: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default = "default_session_target")]
    pub session_target: String,
    #[serde(default = "default_delivery_mode")]
    pub delivery_mode: String,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: i32,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub delete_after_run: bool,
}

fn default_session_target() -> String {
    "main".into()
}
fn default_delivery_mode() -> String {
    "announce".into()
}
fn default_timeout() -> i32 {
    600
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateCronJobRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub schedule_type: Option<String>,
    #[serde(default)]
    pub schedule_value: Option<String>,
    #[serde(default)]
    pub schedule_timezone: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub session_target: Option<String>,
    #[serde(default)]
    pub delivery_mode: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub timeout_seconds: Option<i32>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub delete_after_run: Option<bool>,
}

// ── Handlers ────────────────────────────────────────────────────────

/// GET /v1/agents/{agent_id}/cron — list agent's cron jobs
pub(crate) async fn list_cron_jobs(
    headers: HeaderMap,
    AxumPath(agent_id): AxumPath<String>,
    State(state): State<ApiState>,
) -> Result<Json<Vec<CronJobResponse>>, ApiError> {
    require_agent_workspace_access(&headers, &state, &agent_id).await?;

    let client = state
        .event_store
        .connect()
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("db connect: {e}"))))?;

    let rows = client
        .query(
            "SELECT id, agent_id, conversation_id, name, description,
                    schedule_type, schedule_value, schedule_timezone,
                    message, session_target, delivery_mode, enabled,
                    last_run_at, next_run_at, last_status, last_error,
                    consecutive_errors, timeout_seconds, delete_after_run,
                    created_at, updated_at
             FROM agent_cron_job
             WHERE agent_id = $1
             ORDER BY created_at DESC",
            &[&agent_id],
        )
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("query cron jobs: {e}"))))?;

    let jobs: Vec<CronJobResponse> = rows
        .iter()
        .map(|r| {
            let last_run: Option<chrono::DateTime<chrono::Utc>> = r.get(12);
            let next_run: Option<chrono::DateTime<chrono::Utc>> = r.get(13);
            let created: chrono::DateTime<chrono::Utc> = r.get(19);
            let updated: chrono::DateTime<chrono::Utc> = r.get(20);
            CronJobResponse {
                id: r.get(0),
                agent_id: r.get(1),
                conversation_id: r.get(2),
                name: r.get(3),
                description: r.get(4),
                schedule_type: r.get(5),
                schedule_value: r.get(6),
                schedule_timezone: r.get(7),
                message: r.get(8),
                session_target: r.get(9),
                delivery_mode: r.get(10),
                enabled: r.get(11),
                last_run_at: last_run.map(|t| t.to_rfc3339()),
                next_run_at: next_run.map(|t| t.to_rfc3339()),
                last_status: r.get(14),
                last_error: r.get(15),
                consecutive_errors: r.get(16),
                timeout_seconds: r.get(17),
                delete_after_run: r.get(18),
                created_at: created.to_rfc3339(),
                updated_at: updated.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(jobs))
}

/// POST /v1/agents/{agent_id}/cron — create a cron job
pub(crate) async fn create_cron_job(
    headers: HeaderMap,
    AxumPath(agent_id): AxumPath<String>,
    State(state): State<ApiState>,
    Json(body): Json<CreateCronJobRequest>,
) -> Result<Json<CronJobResponse>, ApiError> {
    let agent_workspace_id = require_agent_workspace_access(&headers, &state, &agent_id).await?;

    if let Some(conversation_id) = body
        .conversation_id
        .as_deref()
        .filter(|conversation_id| !conversation_id.is_empty())
    {
        let conversation = state.db.get_conversation(conversation_id).await?;
        if conversation.workspace_id != agent_workspace_id
            || !conversation.members.contains_key(&agent_id)
        {
            return Err(ApiError::from(AppError::Forbidden(
                "cron conversation must belong to the agent workspace and include the agent".into(),
            )));
        }
    }

    // Validate schedule_type
    if !["at", "every", "cron"].contains(&body.schedule_type.as_str()) {
        return Err(ApiError::from(AppError::Validation(
            "schedule_type must be 'at', 'every', or 'cron'".into(),
        )));
    }

    let conversation_id = body.conversation_id.clone().unwrap_or_default();
    let id = choruz_common::new_id();
    let now = chrono::Utc::now();

    // Compute initial next_run_at
    let next_run = compute_initial_next_run(&body.schedule_type, &body.schedule_value);

    let client = state
        .event_store
        .connect()
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("db connect: {e}"))))?;

    client
        .execute(
            "INSERT INTO agent_cron_job
                (id, agent_id, conversation_id, name, description,
                 schedule_type, schedule_value, schedule_timezone,
                 message, session_target, delivery_mode,
                 timeout_seconds, delete_after_run,
                 next_run_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $15)",
            &[
                &id,
                &agent_id,
                &conversation_id,
                &body.name,
                &body.description,
                &body.schedule_type,
                &body.schedule_value,
                &body.schedule_timezone,
                &body.message,
                &body.session_target,
                &body.delivery_mode,
                &body.timeout_seconds,
                &body.delete_after_run,
                &next_run,
                &now,
            ],
        )
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("insert cron job: {e}"))))?;

    Ok(Json(CronJobResponse {
        id,
        agent_id,
        conversation_id,
        name: body.name,
        description: body.description,
        schedule_type: body.schedule_type,
        schedule_value: body.schedule_value,
        schedule_timezone: body.schedule_timezone,
        message: body.message,
        session_target: body.session_target,
        delivery_mode: body.delivery_mode,
        enabled: true,
        last_run_at: None,
        next_run_at: next_run.map(|t| t.to_rfc3339()),
        last_status: None,
        last_error: None,
        consecutive_errors: 0,
        timeout_seconds: body.timeout_seconds,
        delete_after_run: body.delete_after_run,
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    }))
}

/// PATCH /v1/agents/{agent_id}/cron/{job_id} — update a cron job
///
/// Uses COALESCE-based SQL to apply only the provided fields, avoiding
/// dynamic SQL and `Box<dyn ToSql>` which would make the future !Send.
pub(crate) async fn update_cron_job(
    headers: HeaderMap,
    AxumPath((agent_id, job_id)): AxumPath<(String, String)>,
    State(state): State<ApiState>,
    Json(body): Json<UpdateCronJobRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_agent_workspace_access(&headers, &state, &agent_id).await?;

    let client = state
        .event_store
        .connect()
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("db connect: {e}"))))?;

    // Recompute next_run_at if schedule changed
    let new_next_run: Option<chrono::DateTime<chrono::Utc>> = if body.schedule_type.is_some()
        || body.schedule_value.is_some()
    {
        let current = client
                .query_opt(
                    "SELECT schedule_type, schedule_value FROM agent_cron_job WHERE id = $1 AND agent_id = $2",
                    &[&job_id, &agent_id],
                )
                .await
                .map_err(|e| ApiError::from(AppError::Internal(format!("fetch current: {e}"))))?;
        match current {
            Some(row) => {
                let stype = body
                    .schedule_type
                    .clone()
                    .unwrap_or_else(|| row.get::<_, String>(0));
                let sval = body
                    .schedule_value
                    .clone()
                    .unwrap_or_else(|| row.get::<_, String>(1));
                compute_initial_next_run(&stype, &sval)
            }
            None => {
                return Err(ApiError::from(AppError::NotFound(format!(
                    "cron job {} not found for agent {}",
                    job_id, agent_id
                ))));
            }
        }
    } else {
        None
    };

    // Use COALESCE pattern: each field falls back to existing value if None
    let updated = client
        .execute(
            "UPDATE agent_cron_job SET
                name = COALESCE($3, name),
                schedule_type = COALESCE($4, schedule_type),
                schedule_value = COALESCE($5, schedule_value),
                schedule_timezone = COALESCE($6, schedule_timezone),
                message = COALESCE($7, message),
                session_target = COALESCE($8, session_target),
                delivery_mode = COALESCE($9, delivery_mode),
                enabled = COALESCE($10, enabled),
                timeout_seconds = COALESCE($11, timeout_seconds),
                description = COALESCE($12, description),
                delete_after_run = COALESCE($13, delete_after_run),
                next_run_at = COALESCE($14, next_run_at),
                updated_at = NOW()
             WHERE id = $1 AND agent_id = $2",
            &[
                &job_id,
                &agent_id,
                &body.name,
                &body.schedule_type,
                &body.schedule_value,
                &body.schedule_timezone,
                &body.message,
                &body.session_target,
                &body.delivery_mode,
                &body.enabled,
                &body.timeout_seconds,
                &body.description,
                &body.delete_after_run,
                &new_next_run,
            ],
        )
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("update cron job: {e}"))))?;

    if updated == 0 {
        return Err(ApiError::from(AppError::NotFound(format!(
            "cron job {} not found for agent {}",
            job_id, agent_id
        ))));
    }

    Ok(Json(serde_json::json!({ "updated": true })))
}

/// DELETE /v1/agents/{agent_id}/cron/{job_id} — delete a cron job
pub(crate) async fn delete_cron_job(
    headers: HeaderMap,
    AxumPath((agent_id, job_id)): AxumPath<(String, String)>,
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_agent_workspace_access(&headers, &state, &agent_id).await?;

    let client = state
        .event_store
        .connect()
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("db connect: {e}"))))?;

    let deleted = client
        .execute(
            "DELETE FROM agent_cron_job WHERE id = $1 AND agent_id = $2",
            &[&job_id, &agent_id],
        )
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("delete cron job: {e}"))))?;

    if deleted == 0 {
        return Err(ApiError::from(AppError::NotFound(format!(
            "cron job {} not found for agent {}",
            job_id, agent_id
        ))));
    }

    Ok(Json(serde_json::json!({ "deleted": true })))
}

// ── Helpers ─────────────────────────────────────────────────────────

fn compute_initial_next_run(
    schedule_type: &str,
    schedule_value: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let now = chrono::Utc::now();
    match schedule_type {
        "every" => {
            let duration = parse_interval(schedule_value)?;
            Some(now + duration)
        }
        "cron" => {
            // Approximate: schedule first run soon
            Some(now + chrono::Duration::minutes(1))
        }
        "at" => {
            // Parse ISO datetime
            schedule_value.parse::<chrono::DateTime<chrono::Utc>>().ok()
        }
        _ => None,
    }
}

fn parse_interval(s: &str) -> Option<chrono::Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: i64 = num_str.parse().ok()?;
    match unit {
        "s" => Some(chrono::Duration::seconds(num)),
        "m" => Some(chrono::Duration::minutes(num)),
        "h" => Some(chrono::Duration::hours(num)),
        "d" => Some(chrono::Duration::days(num)),
        _ => {
            let num: i64 = s.parse().ok()?;
            Some(chrono::Duration::minutes(num))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // parse_interval --------------------------------------------------------

    #[test]
    fn parse_interval_returns_none_for_empty_input() {
        assert!(parse_interval("").is_none());
        assert!(parse_interval("   ").is_none());
    }

    #[test]
    fn parse_interval_handles_seconds_minutes_hours_days() {
        assert_eq!(parse_interval("30s"), Some(chrono::Duration::seconds(30)));
        assert_eq!(parse_interval("15m"), Some(chrono::Duration::minutes(15)));
        assert_eq!(parse_interval("2h"), Some(chrono::Duration::hours(2)));
        assert_eq!(parse_interval("7d"), Some(chrono::Duration::days(7)));
    }

    #[test]
    fn parse_interval_trims_whitespace() {
        assert_eq!(
            parse_interval("  10m  "),
            Some(chrono::Duration::minutes(10))
        );
    }

    #[test]
    fn parse_interval_falls_back_to_minutes_for_bare_integers() {
        // No unit suffix → treat as minutes.
        assert_eq!(parse_interval("45"), Some(chrono::Duration::minutes(45)));
    }

    #[test]
    fn parse_interval_returns_none_for_unparseable_input() {
        assert!(parse_interval("abc").is_none());
        assert!(parse_interval("10x").is_none()); // unknown unit AND non-numeric body
        assert!(parse_interval("forty-five").is_none());
    }

    #[test]
    fn parse_interval_accepts_zero_and_negative_intervals() {
        // The function does not impose >0 — callers must validate semantically.
        assert_eq!(parse_interval("0s"), Some(chrono::Duration::seconds(0)));
        assert_eq!(parse_interval("-5m"), Some(chrono::Duration::minutes(-5)));
    }

    // compute_initial_next_run ---------------------------------------------

    #[test]
    fn compute_initial_next_run_every_returns_now_plus_interval() {
        let before = chrono::Utc::now();
        let next = compute_initial_next_run("every", "30s").unwrap();
        let after = chrono::Utc::now();
        // next should be in [before + 30s, after + 30s]
        let lo = before + chrono::Duration::seconds(30);
        let hi = after + chrono::Duration::seconds(30);
        assert!(next >= lo && next <= hi, "{next} not in [{lo}, {hi}]");
    }

    #[test]
    fn compute_initial_next_run_cron_returns_one_minute_from_now() {
        let before = chrono::Utc::now();
        let next = compute_initial_next_run("cron", "0 * * * *").unwrap();
        let delta = next - before;
        assert!(delta >= chrono::Duration::seconds(58));
        assert!(delta <= chrono::Duration::seconds(62));
    }

    #[test]
    fn compute_initial_next_run_at_parses_iso_datetime() {
        let next = compute_initial_next_run("at", "2030-01-01T12:00:00Z").unwrap();
        assert_eq!(next.to_rfc3339(), "2030-01-01T12:00:00+00:00");
    }

    #[test]
    fn compute_initial_next_run_at_returns_none_for_unparseable_value() {
        assert!(compute_initial_next_run("at", "not-a-date").is_none());
    }

    #[test]
    fn compute_initial_next_run_every_returns_none_for_invalid_interval() {
        assert!(compute_initial_next_run("every", "garbage").is_none());
    }

    #[test]
    fn compute_initial_next_run_returns_none_for_unknown_schedule_type() {
        assert!(compute_initial_next_run("hourly", "anything").is_none());
        assert!(compute_initial_next_run("", "30s").is_none());
    }

    // default_* ------------------------------------------------------------

    #[test]
    fn default_session_target_uses_existing_constant() {
        assert!(!default_session_target().is_empty());
    }

    #[test]
    fn default_delivery_mode_uses_existing_constant() {
        assert!(!default_delivery_mode().is_empty());
    }

    #[test]
    fn default_timeout_is_positive() {
        assert!(default_timeout() > 0);
    }
}
