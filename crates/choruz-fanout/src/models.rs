//! Domain models for the Fanout Gateway.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Client Cursor
// ---------------------------------------------------------------------------

/// A client cursor tracks the last sequence number a client has received
/// for a given conversation. Used for replay on reconnect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCursor {
    pub client_id: String,
    pub conversation_id: String,
    pub last_seen_seq: i64,
    pub updated_at: DateTime<Utc>,
}

impl ClientCursor {
    /// Create a new cursor starting at sequence 0 (no events seen).
    pub fn new(client_id: impl Into<String>, conversation_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            conversation_id: conversation_id.into(),
            last_seen_seq: 0,
            updated_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Subscription
// ---------------------------------------------------------------------------

/// Represents an active client subscription to a conversation.
#[derive(Debug, Clone)]
pub struct Subscription {
    pub client_id: String,
    pub conversation_id: String,
    /// The sequence number of the last event pushed to this client.
    pub last_pushed_seq: i64,
}

// ---------------------------------------------------------------------------
// Fan-out Event (wire format sent to clients)
// ---------------------------------------------------------------------------

/// An event pushed to clients over WS/SSE.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanoutEvent {
    pub conversation_id: String,
    pub seq: i64,
    pub event_id: String,
    pub event_type: String,
    pub sender_id: String,
    pub content: Option<String>,
    pub content_type: String,
    pub metadata: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_msg_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_cursor_new() {
        let cursor = ClientCursor::new("client-1", "conv-1");
        assert_eq!(cursor.client_id, "client-1");
        assert_eq!(cursor.conversation_id, "conv-1");
        assert_eq!(cursor.last_seen_seq, 0);
    }

    #[test]
    fn client_cursor_serde_roundtrip() {
        let cursor = ClientCursor::new("client-1", "conv-1");
        let json = serde_json::to_string(&cursor).unwrap();
        let parsed: ClientCursor = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.client_id, "client-1");
        assert_eq!(parsed.last_seen_seq, 0);
    }

    #[test]
    fn fanout_event_serde_roundtrip() {
        let event = FanoutEvent {
            conversation_id: "conv-1".into(),
            seq: 42,
            event_id: "evt-1".into(),
            event_type: "message".into(),
            sender_id: "user-1".into(),
            content: Some("hello".into()),
            content_type: "text/plain".into(),
            metadata: serde_json::json!({}),
            client_msg_id: None,
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: FanoutEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.seq, 42);
        assert_eq!(parsed.event_type, "message");
    }

    #[test]
    fn fanout_event_with_none_content() {
        let event = FanoutEvent {
            conversation_id: "conv-1".into(),
            seq: 1,
            event_id: "evt-1".into(),
            event_type: "system".into(),
            sender_id: "system".into(),
            content: None,
            content_type: "text/plain".into(),
            metadata: serde_json::json!({"action": "member_joined"}),
            client_msg_id: None,
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: FanoutEvent = serde_json::from_str(&json).unwrap();
        assert!(parsed.content.is_none());
    }
}
