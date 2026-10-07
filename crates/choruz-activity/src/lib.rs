//! Storage-free activity event validation and defensive redaction.
use choruz_common::AppError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod redaction;
pub use redaction::{redact_sensitive_text, sanitize_value};

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

/// Validate a batch before persistence. Sanitize each data payload first.
/// Returns a validation error for invalid schema, identifiers, bounds or payloads.
pub fn validate_batch(events: &[TelemetryEvent]) -> Result<(), AppError> {
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
    Ok(())
}
