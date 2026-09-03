//! Unified event envelope for all bus messages.
//!
//! Every event flowing through the durable bus (Kafka or in-process channel) is
//! wrapped in an [`EventEnvelope`].  The envelope carries routing metadata and
//! the strongly-typed payload is serialised inside `payload` as JSON.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Schema version for the envelope format itself.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Unified event envelope.
///
/// All bus messages share this outer structure.  Consumers deserialise the
/// envelope first, then dispatch on `event_type` / `aggregate_type` to
/// parse `payload` into the concrete topic struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// Globally unique event ID.
    pub event_id: String,

    /// Discriminator for the payload type (e.g. `"message"`, `"reply"`,
    /// `"agent_command"`, `"agent_result"`, `"tool_effect"`).
    pub event_type: String,

    /// The domain aggregate this event belongs to
    /// (e.g. `"conversation_event"`, `"agent_command"`, `"agent_result"`).
    pub aggregate_type: String,

    /// Aggregate ID — used as the Kafka partition key so that events for the
    /// same aggregate land on the same partition.
    pub aggregate_id: String,

    /// Serialised payload.  Consumers further deserialise this into the
    /// concrete topic-specific struct.
    pub payload: serde_json::Value,

    /// Distributed trace ID for end-to-end observability.
    pub trace_id: String,

    /// Tenant / organisation ID.
    pub org_id: String,

    /// When this event was produced.
    pub timestamp: DateTime<Utc>,

    /// Schema version — lets consumers handle backward-compatible evolution.
    pub schema_version: u32,
}

impl EventEnvelope {
    /// Create a new envelope with the current schema version and `Utc::now()`
    /// timestamp.
    #[must_use]
    pub fn new(
        event_id: impl Into<String>,
        event_type: impl Into<String>,
        aggregate_type: impl Into<String>,
        aggregate_id: impl Into<String>,
        payload: serde_json::Value,
        trace_id: impl Into<String>,
        org_id: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            event_type: event_type.into(),
            aggregate_type: aggregate_type.into(),
            aggregate_id: aggregate_id.into(),
            payload,
            trace_id: trace_id.into(),
            org_id: org_id.into(),
            timestamp: Utc::now(),
            schema_version: CURRENT_SCHEMA_VERSION,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_serde_roundtrip() {
        let env = EventEnvelope::new(
            "evt-1",
            "message",
            "conversation_event",
            "conv-abc",
            serde_json::json!({"hello": "world"}),
            "trace-1",
            "org-1",
        );

        let json = serde_json::to_string(&env).expect("serialize");
        let parsed: EventEnvelope = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.event_id, "evt-1");
        assert_eq!(parsed.event_type, "message");
        assert_eq!(parsed.schema_version, CURRENT_SCHEMA_VERSION);
    }

    /// Envelope timestamp is set to approximately now.
    #[test]
    fn envelope_timestamp_is_recent() {
        let before = Utc::now();
        let env = EventEnvelope::new(
            "evt-2",
            "reply",
            "conversation_event",
            "conv-xyz",
            serde_json::json!(null),
            "trace-2",
            "org-2",
        );
        let after = Utc::now();

        assert!(env.timestamp >= before);
        assert!(env.timestamp <= after);
    }

    /// Envelope with null payload roundtrips.
    #[test]
    fn envelope_null_payload_roundtrip() {
        let env = EventEnvelope::new(
            "evt-3",
            "system",
            "system_event",
            "global",
            serde_json::Value::Null,
            "trace-3",
            "org-3",
        );

        let json = serde_json::to_string(&env).expect("serialize");
        let parsed: EventEnvelope = serde_json::from_str(&json).expect("deserialize");
        assert!(parsed.payload.is_null());
    }

    /// Envelope with empty object payload.
    #[test]
    fn envelope_empty_object_payload() {
        let env = EventEnvelope::new(
            "evt-4",
            "heartbeat",
            "system_event",
            "node-1",
            serde_json::json!({}),
            "trace-4",
            "org-4",
        );

        let json = serde_json::to_string(&env).expect("serialize");
        let parsed: EventEnvelope = serde_json::from_str(&json).expect("deserialize");
        assert!(parsed.payload.is_object());
        assert_eq!(parsed.payload.as_object().unwrap().len(), 0);
    }

    /// Envelope with large nested payload.
    #[test]
    fn envelope_large_nested_payload() {
        let big_array: Vec<i32> = (0..1000).collect();
        let env = EventEnvelope::new(
            "evt-5",
            "batch",
            "conversation_event",
            "conv-big",
            serde_json::json!({"items": big_array}),
            "trace-5",
            "org-5",
        );

        let json = serde_json::to_string(&env).expect("serialize");
        let parsed: EventEnvelope = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.payload["items"].as_array().unwrap().len(), 1000);
    }

    /// Envelope preserves all fields through clone.
    #[test]
    fn envelope_clone_preserves_all_fields() {
        let env = EventEnvelope::new(
            "evt-6",
            "message",
            "conversation_event",
            "conv-clone",
            serde_json::json!({"key": "value"}),
            "trace-6",
            "org-6",
        );
        let cloned = env.clone();

        assert_eq!(cloned.event_id, env.event_id);
        assert_eq!(cloned.event_type, env.event_type);
        assert_eq!(cloned.aggregate_type, env.aggregate_type);
        assert_eq!(cloned.aggregate_id, env.aggregate_id);
        assert_eq!(cloned.trace_id, env.trace_id);
        assert_eq!(cloned.org_id, env.org_id);
        assert_eq!(cloned.schema_version, env.schema_version);
        assert_eq!(cloned.timestamp, env.timestamp);
        assert_eq!(cloned.payload, env.payload);
    }

    /// Deserializing from JSON with a missing required field fails.
    #[test]
    fn envelope_missing_field_fails() {
        let json = r#"{"event_id":"e","event_type":"t"}"#;
        let result: Result<EventEnvelope, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }
}
