use choruz_common::AppError;
use choruz_domain::Principal;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::DbService;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TelemetryEvent {
    pub event_id: String,
    pub schema_version: i32,
    pub trace_id: String,
    pub span_id: String,
    pub session_id: String,
    pub name: String,
    pub ts: DateTime<Utc>,
    pub duration_ms: Option<i64>,
    pub data: Option<Value>,
}

impl DbService {
    /// Commits a validated, sanitized batch atomically. Event identity is scoped
    /// to the authenticated principal; acknowledgement can safely be retried.
    pub async fn record_telemetry(
        &self,
        principal: &Principal,
        events: &[TelemetryEvent],
    ) -> Result<(), AppError> {
        if events.is_empty() || events.len() > 100 {
            return Err(AppError::Validation(
                "activity batches require 1–100 events".into(),
            ));
        }
        for event in events {
            let identifiers = [
                &event.event_id,
                &event.trace_id,
                &event.span_id,
                &event.session_id,
                &event.name,
            ];
            if event.schema_version != 1
                || identifiers.iter().any(|s| {
                    s.is_empty()
                        || s.len() > 128
                        || !s
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
                })
                || event.duration_ms.is_some_and(|d| d < 0)
                || event
                    .data
                    .as_ref()
                    .is_some_and(|d| !d.is_object() || d.to_string().len() > 16_384)
            {
                return Err(AppError::Validation(
                    "invalid activity event fields or payload size".into(),
                ));
            }
        }
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("begin activity batch: {e}")))?;
        for event in events {
            tx.execute(
                "INSERT INTO telemetry_event (workspace_id, principal_id, event_id, schema_version, trace_id, span_id, session_id, name, occurred_at, duration_ms, data)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
                 ON CONFLICT (workspace_id, principal_id, event_id) DO NOTHING",
                &[&principal.workspace_id, &principal.id, &event.event_id, &event.schema_version, &event.trace_id, &event.span_id, &event.session_id, &event.name, &event.ts, &event.duration_ms, &event.data],
            ).await.map_err(|e| AppError::Internal(format!("persist activity batch: {e}")))?;
        }
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("commit activity batch: {e}")))?;
        for event in events {
            tracing::debug!(event_id = %event.event_id, trace_id = %event.trace_id, span_id = %event.span_id, name = %event.name, principal_id = %principal.id, "activity persisted or replayed");
        }
        tracing::info!(principal_id = %principal.id, count = events.len(), "activity batch committed");
        Ok(())
    }
}
