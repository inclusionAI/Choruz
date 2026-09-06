use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use choruz_application::db_service::{ActivityCursor, ActivityFilter, ActivitySource};
use choruz_common::AppError;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{ApiError, ApiState, require_human_operator};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActivityQuery {
    #[serde(default)]
    source: ActivitySource,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    trace_id: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

impl ActivityQuery {
    fn filter(&self) -> ActivityFilter {
        ActivityFilter {
            source: self.source,
            since: self.since,
            until: self.until,
            trace_id: self.trace_id.clone(),
        }
    }
}

pub(crate) async fn list(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Query(query): Query<ActivityQuery>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    let cursor: Option<ActivityCursor> = query
        .cursor
        .as_ref()
        .map(|value| {
            if value.len() > 1024 {
                return Err(AppError::Validation("invalid activity cursor".into()));
            }
            URL_SAFE_NO_PAD
                .decode(value)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .ok_or_else(|| AppError::Validation("invalid activity cursor".into()))
        })
        .transpose()?;
    let (records, next) = state
        .db
        .activity_page(
            &actor,
            &query.filter(),
            cursor.as_ref(),
            query.limit.unwrap_or(100),
        )
        .await?;
    let records: Vec<_> = records
        .into_iter()
        .map(crate::handlers_events::sanitize_telemetry_value)
        .collect();
    let next_cursor = next
        .map(|c| serde_json::to_vec(&c).map(|v| URL_SAFE_NO_PAD.encode(v)))
        .transpose()
        .map_err(|e| AppError::Internal(format!("encode activity cursor: {e}")))?;
    state
        .db
        .record_audit(
            &actor.workspace_id,
            &actor.id,
            "activity.read",
            "principal",
            &actor.id,
            json!({"source":query.source,"count":records.len()}),
        )
        .await?;
    Ok(Json(json!({"records":records,"next_cursor":next_cursor})))
}

pub(crate) async fn summary(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Query(query): Query<ActivityQuery>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    if query.cursor.is_some() || query.limit.is_some() {
        return Err(
            AppError::Validation("summary does not accept pagination parameters".into()).into(),
        );
    }
    let value = state.db.activity_summary(&actor, &query.filter()).await?;
    state
        .db
        .record_audit(
            &actor.workspace_id,
            &actor.id,
            "activity.summary",
            "principal",
            &actor.id,
            json!({"source":query.source}),
        )
        .await?;
    Ok(Json(value))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PruneRequest {
    before: DateTime<Utc>,
    #[serde(default)]
    apply: bool,
}

pub(crate) async fn prune(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(body): Json<PruneRequest>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    Ok(Json(
        state
            .db
            .prune_activity(&actor, body.before, body.apply)
            .await?,
    ))
}
