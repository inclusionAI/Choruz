use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use choruz_application::ListEventsQuery;
use serde::Deserialize;
use serde_json::json;

use crate::{
    ApiError, ApiState, authenticated_principal, db_persist, flush_webhooks, flush_webhooks_all,
    require_actor, require_self,
};

// ── Telemetry ─────────────────────────────────────────────────────────
static ACTIVITY_BATCHES: std::sync::LazyLock<choruz_common::metrics::IntCounterVec> =
    std::sync::LazyLock::new(|| {
        choruz_common::metrics::register_counter_vec(
            "choruz_activity_batches_total",
            "Authenticated activity batch persistence attempts.",
            &["outcome"],
        )
    });
#[derive(Debug, Deserialize)]
pub(crate) struct TelemetryPayload {
    events: Vec<choruz_activity::TelemetryEvent>,
}

pub(crate) async fn ingest_telemetry(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(mut payload): Json<TelemetryPayload>,
) -> Result<StatusCode, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    for event in &mut payload.events {
        event.data = event.data.take().map(choruz_activity::sanitize_value);
    }
    let result = state.db.record_telemetry(&principal, &payload.events).await;
    ACTIVITY_BATCHES
        .with_label_values(&[if result.is_ok() {
            "committed"
        } else {
            "failed"
        }])
        .inc();
    result?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Events ────────────────────────────────────────────────────────────

pub(crate) async fn list_events(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(principal_id): Path<String>,
    Query(query): Query<ListEventsQuery>,
) -> Result<Json<Vec<choruz_domain::EventEnvelope>>, ApiError> {
    require_self(&headers, &state, &principal_id).await?;
    // Phase 4: read events from DB instead of in-memory ChatApp
    let events = state
        .db
        .list_events(&principal_id, query.cursor, None)
        .await
        .map_err(ApiError)?;
    Ok(Json(events))
}

pub(crate) async fn ack_events(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(principal_id): Path<String>,
    Json(payload): Json<choruz_application::AckEventsRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_self(&headers, &state, &principal_id).await?;
    // Phase 4: acknowledge events in DB instead of in-memory ChatApp
    let ack_cursor = state
        .db
        .ack_events(&principal_id, payload.upto_delivery_seq)
        .await
        .map_err(ApiError)?;

    Ok(Json(json!({
        "ack_cursor": ack_cursor
    })))
}

pub(crate) async fn set_event_webhook(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(principal_id): Path<String>,
    Json(payload): Json<choruz_application::SetEventWebhookRequest>,
) -> Result<Json<choruz_application::EventWebhookConfig>, ApiError> {
    require_actor(&headers, &state, &payload.actor_id).await?;
    let config = state.app.set_event_webhook(&principal_id, payload)?;
    let _ = flush_webhooks(&state.app).await;

    // Persist webhook config to DB
    {
        let event_types: Vec<&str> = config.event_types.iter().map(|s| s.as_str()).collect();
        let cursor = config.cursor as i64;
        db_persist(
            &state.event_store,
            "INSERT INTO event_webhook (principal_id, url, event_types, cursor, webhook_secret, updated_at)
             VALUES ($1, $2, $3, $4, $5, NOW())
             ON CONFLICT (principal_id)
             DO UPDATE SET url = EXCLUDED.url, event_types = EXCLUDED.event_types,
                           cursor = EXCLUDED.cursor, webhook_secret = EXCLUDED.webhook_secret,
                           updated_at = NOW()",
            &[
                &config.principal_id,
                &config.url,
                &event_types,
                &cursor,
                &config.webhook_secret,
            ],
            "set_event_webhook",
        ).await;
    }

    Ok(Json(config))
}

// ── Webhook flush ─────────────────────────────────────────────────────

#[allow(dead_code)]
pub(crate) async fn flush_webhook_deliveries(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<Json<crate::WebhookFlushResponse>, ApiError> {
    authenticated_principal(&headers, &state).await?;
    let response = flush_webhooks_all(&state.app, &state.db).await;

    Ok(Json(response))
}

// Old /v1/ws/events WebSocket endpoint removed.
// Dashboard changes are delivered by handlers_sync_ws through /v1/ws/sync.
