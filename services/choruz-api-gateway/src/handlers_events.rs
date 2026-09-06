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
    redact_sensitive_text, require_actor, require_self,
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
    events: Vec<choruz_application::db_service::TelemetryEvent>,
}

pub(crate) async fn ingest_telemetry(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(mut payload): Json<TelemetryPayload>,
) -> Result<StatusCode, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    for event in &mut payload.events {
        event.data = event.data.take().map(sanitize_telemetry_value);
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

pub(crate) fn sanitize_telemetry_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(mut object) => {
            let private_payload = object
                .get("private")
                .or_else(|| object.get("is_private"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
                || object.get("privacy").and_then(|value| value.as_str()) == Some("private");

            for (key, value) in object.iter_mut() {
                if telemetry_key_is_sensitive(key)
                    || (private_payload && telemetry_key_is_private_content(key))
                {
                    *value = serde_json::Value::String("[REDACTED]".into());
                } else {
                    *value = sanitize_telemetry_value(value.take());
                }
            }

            serde_json::Value::Object(object)
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(sanitize_telemetry_value).collect())
        }
        serde_json::Value::String(value) => {
            serde_json::Value::String(redact_sensitive_text(&value))
        }
        other => other,
    }
}

fn telemetry_key_is_sensitive(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    let compact_key: String = key.chars().filter(|ch| *ch != '_' && *ch != '-').collect();
    key == "authorization"
        || key == "cookie"
        || key == "set-cookie"
        || matches!(
            compact_key.as_str(),
            "authorization"
                | "cookie"
                | "setcookie"
                | "database64"
                | "attachmentbytes"
                | "filebytes"
                | "contentbytes"
                | "bodybytes"
                | "rawbytes"
                | "bytesbase64"
                | "payloadbase64"
                | "filename"
                | "attachmentname"
                | "path"
                | "paths"
                | "authenticationcode"
                | "authorizationcode"
                | "devicecode"
                | "pairingcredential"
                | "credential"
        )
        || compact_key.ends_with("filename")
        || key.contains("secret")
        || compact_key.contains("secret")
        || key.contains("password")
        || compact_key.contains("password")
        || key.ends_with("_path")
        || key.ends_with("_paths")
        || compact_key.ends_with("path")
        || compact_key.ends_with("paths")
        || key.contains("session_token")
        || compact_key.contains("sessiontoken")
        || key.ends_with("_token")
        || key.ends_with("token")
        || compact_key.ends_with("token")
}

fn telemetry_key_is_private_content(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "content" | "message" | "text" | "body" | "preview"
    )
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
