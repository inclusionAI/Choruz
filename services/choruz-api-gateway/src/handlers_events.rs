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

#[derive(Debug, Deserialize)]
pub(crate) struct TelemetryPayload {
    events: Vec<serde_json::Value>,
}

pub(crate) async fn ingest_telemetry(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(payload): Json<TelemetryPayload>,
) -> Result<StatusCode, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;

    // Persist to DB so telemetry is queryable, AND emit a structured log
    // for every event so FE telemetry ends up in the same grep-able stream
    // as backend span logs. Without the second step, "logs alone" could not
    // reach any FE event; a production engineer had to know to query
    // `telemetry_event` separately to see clicks / pixel_world transitions
    // / agent_reply events.
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    for evt in &payload.events {
        let name = evt.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let trace_id = evt.get("traceId").and_then(|v| v.as_str());
        let duration = evt.get("durationMs").and_then(|v| v.as_i64());
        let data = evt.get("data").cloned().map(sanitize_telemetry_value);

        // Structured log — only a small allowlist of known-safe correlation
        // fields is promoted to stdout; everything else is represented as
        // just its key list. Without this split, logging the full `data`
        // payload (as round 4 originally did) widens PII exposure from DB
        // to centralized logs — search queries, file paths, company names,
        // WS frame previews, agent display names all ride in `data`.
        //
        // What counts as "safe" here = opaque identifiers or small
        // enumerations that the FE demonstrably sets to non-user-typed
        // values. Anything a user can shape (names, queries, free text)
        // is intentionally excluded.
        let data_keys: Vec<String> = data
            .as_ref()
            .and_then(|v| v.as_object())
            .map(|obj| obj.keys().cloned().collect())
            .unwrap_or_default();
        let data_len = data.as_ref().map(|v| v.to_string().len()).unwrap_or(0);
        let pick = |k: &str| -> Option<String> {
            data.as_ref()
                .and_then(|v| v.get(k))
                .and_then(|v| v.as_str().map(|s| s.to_string()))
        };
        let backend_trace_id = pick("backend_trace_id");
        let source = pick("source");
        let src_conversation_id = pick("conversation_id");
        let src_agent_id = pick("agent_id");
        let src_message_id = pick("message_id").or_else(|| pick("event_id"));
        let src_pixel_world_instance_id = pick("pixel_world_instance_id");
        let src_from_state = pick("from_state");
        let src_to_state = pick("to_state");
        let src_arrival_state = pick("arrival_state");
        let src_resume_state = pick("resume_state");
        let content_len = data
            .as_ref()
            .and_then(|v| v.get("content_len"))
            .and_then(|v| v.as_i64());
        tracing::info!(
            event = "fe_telemetry",
            source = "fe",
            name = %name,
            trace_id = trace_id.unwrap_or("none"),
            principal_id = %principal.id,
            duration_ms = duration.unwrap_or(-1),
            backend_trace_id = backend_trace_id.as_deref().unwrap_or("none"),
            fe_source = source.as_deref().unwrap_or("-"),
            conversation_id = src_conversation_id.as_deref().unwrap_or("-"),
            agent_id = src_agent_id.as_deref().unwrap_or("-"),
            message_id = src_message_id.as_deref().unwrap_or("-"),
            pixel_world_instance_id = src_pixel_world_instance_id.as_deref().unwrap_or("-"),
            from_state = src_from_state.as_deref().unwrap_or("-"),
            to_state = src_to_state.as_deref().unwrap_or("-"),
            arrival_state = src_arrival_state.as_deref().unwrap_or("-"),
            resume_state = src_resume_state.as_deref().unwrap_or("-"),
            content_len = content_len.unwrap_or(-1),
            // Key list covers any non-allowlisted field the FE emitted, so
            // operators can still spot schema drift without leaking values.
            data_keys = ?data_keys,
            data_len,
            "frontend telemetry event"
        );

        if let Err(e) = client
            .execute(
                "INSERT INTO telemetry_event (principal_id, trace_id, name, duration_ms, data)
             VALUES ($1, $2, $3, $4, $5)",
                &[&principal.id, &trace_id, &name, &duration, &data],
            )
            .await
        {
            tracing::warn!(error = %e, name, "telemetry DB write failed (non-fatal)");
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

fn sanitize_telemetry_value(value: serde_json::Value) -> serde_json::Value {
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
// Event push is handled by choruz-fanout (/ws/fanout on pipeline port).
