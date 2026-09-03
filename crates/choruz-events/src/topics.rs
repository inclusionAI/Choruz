//! Topic-specific payload structs.
//!
//! Each struct corresponds to the strongly-typed payload that rides inside an
//! [`EventEnvelope::payload`](super::envelope::EventEnvelope).
//!
//! The field types intentionally use `String` for IDs that cross the
//! serialisation boundary — callers convert to/from `choruz_ids` types at the
//! application edge.  This keeps the wire format stable even if the newtype
//! internals evolve.

use choruz_ids::{
    AttemptId, CommandId, DeliveryId, MessageId, ReplyEventId, RouteId, ToolCallId, TurnId,
};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// conversation_events topic
// ---------------------------------------------------------------------------

/// Sender type discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SenderType {
    User,
    Agent,
    System,
}

/// Event type discriminator for conversation events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationEventType {
    Message,
    Reply,
    Reaction,
    Edit,
    System,
}

/// Payload published to the `conversation_events` Kafka topic.
///
/// Produced by the Ingress API (for user messages) and the Conversation Writer
/// (for agent replies).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationEventPayload {
    pub message_id: MessageId,
    pub conversation_id: String,
    pub seq: i64,
    pub sender_id: String,
    pub sender_type: SenderType,
    pub content: Option<String>,
    pub content_type: String,
    pub event_type: ConversationEventType,
    /// Present on user-originated messages (retry dedup).
    pub client_msg_id: Option<String>,
    /// Present on agent reply events.
    pub turn_id: Option<TurnId>,
    /// Present on agent reply events.
    pub reply_event_id: Option<ReplyEventId>,
    pub metadata: serde_json::Value,
}

// ---------------------------------------------------------------------------
// route_decisions topic
// ---------------------------------------------------------------------------

/// Outcome of a routing decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteDecisionOutcome {
    Trigger,
    Skip,
    Error,
}

/// Payload published to the `route_decisions` topic.
///
/// One record per (message, agent) pair — always produced, even for "skip",
/// so the decision is auditable in the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecisionPayload {
    pub route_id: RouteId,
    pub message_id: MessageId,
    pub agent_id: String,
    pub conversation_id: String,
    pub decision: RouteDecisionOutcome,
    pub reason: String,
    pub policy_snapshot: serde_json::Value,
}

// ---------------------------------------------------------------------------
// agent_commands topic
// ---------------------------------------------------------------------------

/// Priority levels for agent commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum CommandPriority {
    Normal = 0,
    High = 1,
    Urgent = 2,
}

/// Payload published to the `agent_commands` topic.
///
/// Consumed by the Session Manager, which assigns an executor and lease.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCommandPayload {
    pub command_id: CommandId,
    pub route_id: RouteId,
    pub session_key: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub message_id: MessageId,
    pub turn_id: TurnId,
    pub prompt: String,
    pub priority: CommandPriority,
    pub metadata: serde_json::Value,
}

// ---------------------------------------------------------------------------
// agent_results topic
// ---------------------------------------------------------------------------

/// Status of an agent execution attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentResultStatus {
    Succeeded,
    Failed,
}

/// Payload published to the `agent_results` topic.
///
/// Consumed by the Conversation Writer, which commits the first successful
/// result for a given `turn_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResultPayload {
    pub turn_id: TurnId,
    pub attempt_id: AttemptId,
    pub command_id: CommandId,
    pub session_key: String,
    pub conversation_id: String,
    pub agent_id: String,
    pub status: AgentResultStatus,
    pub content: Option<String>,
    pub content_type: Option<String>,
    pub error: Option<String>,
    pub tool_calls_count: i32,
    pub execution_duration_ms: i64,
}

// ---------------------------------------------------------------------------
// tool_effects topic
// ---------------------------------------------------------------------------

/// Status of a tool effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffectStatus {
    Pending,
    Executing,
    Succeeded,
    Failed,
}

/// Payload published to the `tool_effects` topic.
///
/// Recorded by the Tool Gateway for observability and idempotent replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEffectPayload {
    pub tool_call_id: ToolCallId,
    pub attempt_id: AttemptId,
    pub turn_id: TurnId,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub tool_output: Option<serde_json::Value>,
    pub status: ToolEffectStatus,
    pub is_mutating: bool,
    pub external_idempotency_key: Option<String>,
}

// ---------------------------------------------------------------------------
// System-level events (dead letters, delivery confirmations, etc.)
// ---------------------------------------------------------------------------

/// Source type for dead-lettered items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadLetterSourceType {
    Command,
    ToolEffect,
    Delivery,
}

/// A dead-lettered event that could not be processed after max retries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadLetterPayload {
    pub source_type: DeadLetterSourceType,
    pub source_id: String,
    pub payload: serde_json::Value,
    pub error: String,
    pub attempt_count: i32,
}

/// A delivery confirmation for fanout to a client sink.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryPayload {
    pub delivery_id: DeliveryId,
    pub reply_event_id: ReplyEventId,
    pub conversation_id: String,
    pub recipient_id: String,
    pub channel: String,
    pub status: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_event_payload_roundtrip() {
        let payload = ConversationEventPayload {
            message_id: MessageId::new(),
            conversation_id: "conv-1".into(),
            seq: 42,
            sender_id: "user-1".into(),
            sender_type: SenderType::User,
            content: Some("hello".into()),
            content_type: "text".into(),
            event_type: ConversationEventType::Message,
            client_msg_id: Some("client-1".into()),
            turn_id: None,
            reply_event_id: None,
            metadata: serde_json::json!({}),
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        let parsed: ConversationEventPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.seq, 42);
        assert_eq!(parsed.sender_type, SenderType::User);
        assert_eq!(parsed.event_type, ConversationEventType::Message);
    }

    #[test]
    fn agent_command_payload_roundtrip() {
        let payload = AgentCommandPayload {
            command_id: CommandId::new(),
            route_id: RouteId::new(),
            session_key: "agent-1:conv-1".into(),
            agent_id: "agent-1".into(),
            conversation_id: "conv-1".into(),
            message_id: MessageId::new(),
            turn_id: TurnId::new(),
            prompt: "review this PR".into(),
            priority: CommandPriority::Normal,
            metadata: serde_json::json!({"lang": "rust"}),
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        let parsed: AgentCommandPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.session_key, "agent-1:conv-1");
        assert_eq!(parsed.priority, CommandPriority::Normal);
    }

    #[test]
    fn agent_result_payload_roundtrip() {
        let payload = AgentResultPayload {
            turn_id: TurnId::new(),
            attempt_id: AttemptId::new(),
            command_id: CommandId::new(),
            session_key: "agent-1:conv-1".into(),
            conversation_id: "conv-1".into(),
            agent_id: "agent-1".into(),
            status: AgentResultStatus::Succeeded,
            content: Some("LGTM".into()),
            content_type: Some("text".into()),
            error: None,
            tool_calls_count: 3,
            execution_duration_ms: 4500,
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        let parsed: AgentResultPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.status, AgentResultStatus::Succeeded);
        assert_eq!(parsed.tool_calls_count, 3);
    }

    #[test]
    fn tool_effect_payload_roundtrip() {
        let payload = ToolEffectPayload {
            tool_call_id: ToolCallId::new(),
            attempt_id: AttemptId::new(),
            turn_id: TurnId::new(),
            tool_name: "github_comment".into(),
            tool_input: serde_json::json!({"pr": 42}),
            tool_output: Some(serde_json::json!({"id": 999})),
            status: ToolEffectStatus::Succeeded,
            is_mutating: true,
            external_idempotency_key: Some("echat:tc-abc".into()),
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        let parsed: ToolEffectPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.tool_name, "github_comment");
        assert!(parsed.is_mutating);
        assert_eq!(parsed.status, ToolEffectStatus::Succeeded);
    }

    #[test]
    fn route_decision_payload_roundtrip() {
        let payload = RouteDecisionPayload {
            route_id: RouteId::new(),
            message_id: MessageId::new(),
            agent_id: "agent-1".into(),
            conversation_id: "conv-1".into(),
            decision: RouteDecisionOutcome::Skip,
            reason: "not_mentioned".into(),
            policy_snapshot: serde_json::json!({"mode": "mentioned_only"}),
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        let parsed: RouteDecisionPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.decision, RouteDecisionOutcome::Skip);
        assert_eq!(parsed.reason, "not_mentioned");
    }

    #[test]
    fn dead_letter_payload_roundtrip() {
        let payload = DeadLetterPayload {
            source_type: DeadLetterSourceType::Command,
            source_id: "cmd-1".into(),
            payload: serde_json::json!({"original": true}),
            error: "timeout after 5 attempts".into(),
            attempt_count: 5,
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        let parsed: DeadLetterPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.source_type, DeadLetterSourceType::Command);
        assert_eq!(parsed.attempt_count, 5);
    }

    // -----------------------------------------------------------------------
    // Edge-case tests
    // -----------------------------------------------------------------------

    /// DeliveryPayload roundtrip (was missing from original tests).
    #[test]
    fn delivery_payload_roundtrip() {
        let payload = DeliveryPayload {
            delivery_id: DeliveryId::new(),
            reply_event_id: ReplyEventId::new(),
            conversation_id: "conv-1".into(),
            recipient_id: "user-1".into(),
            channel: "websocket".into(),
            status: "delivered".into(),
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        let parsed: DeliveryPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.conversation_id, "conv-1");
        assert_eq!(parsed.channel, "websocket");
    }

    /// Conversation event as agent reply (turn_id and reply_event_id set).
    #[test]
    fn conversation_event_as_agent_reply() {
        let turn = TurnId::new();
        let reply = ReplyEventId::new();
        let payload = ConversationEventPayload {
            message_id: MessageId::new(),
            conversation_id: "conv-2".into(),
            seq: 100,
            sender_id: "agent-backend".into(),
            sender_type: SenderType::Agent,
            content: Some("LGTM, merging.".into()),
            content_type: "text".into(),
            event_type: ConversationEventType::Reply,
            client_msg_id: None,
            turn_id: Some(turn),
            reply_event_id: Some(reply),
            metadata: serde_json::json!({"confidence": 0.95}),
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        let parsed: ConversationEventPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.sender_type, SenderType::Agent);
        assert_eq!(parsed.event_type, ConversationEventType::Reply);
        assert_eq!(parsed.turn_id, Some(turn));
        assert_eq!(parsed.reply_event_id, Some(reply));
        assert!(parsed.client_msg_id.is_none());
    }

    /// Conversation event with content=None (e.g., reaction or system event).
    #[test]
    fn conversation_event_with_none_content() {
        let payload = ConversationEventPayload {
            message_id: MessageId::new(),
            conversation_id: "conv-3".into(),
            seq: 1,
            sender_id: "system".into(),
            sender_type: SenderType::System,
            content: None,
            content_type: "text".into(),
            event_type: ConversationEventType::System,
            client_msg_id: None,
            turn_id: None,
            reply_event_id: None,
            metadata: serde_json::json!({"action": "member_joined"}),
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        let parsed: ConversationEventPayload = serde_json::from_str(&json).expect("deserialize");
        assert!(parsed.content.is_none());
        assert_eq!(parsed.sender_type, SenderType::System);
        assert_eq!(parsed.event_type, ConversationEventType::System);
    }

    /// Agent result with Failed status and error message.
    #[test]
    fn agent_result_payload_failed_with_error() {
        let payload = AgentResultPayload {
            turn_id: TurnId::new(),
            attempt_id: AttemptId::new(),
            command_id: CommandId::new(),
            session_key: "agent-1:conv-1".into(),
            conversation_id: "conv-1".into(),
            agent_id: "agent-1".into(),
            status: AgentResultStatus::Failed,
            content: None,
            content_type: None,
            error: Some("process exited with code 1".into()),
            tool_calls_count: 0,
            execution_duration_ms: 120_000,
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        let parsed: AgentResultPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.status, AgentResultStatus::Failed);
        assert!(parsed.content.is_none());
        assert_eq!(parsed.error, Some("process exited with code 1".into()));
    }

    /// Tool effect in Pending status with no output yet.
    #[test]
    fn tool_effect_pending_no_output() {
        let payload = ToolEffectPayload {
            tool_call_id: ToolCallId::new(),
            attempt_id: AttemptId::new(),
            turn_id: TurnId::new(),
            tool_name: "git_push".into(),
            tool_input: serde_json::json!({"branch": "main"}),
            tool_output: None,
            status: ToolEffectStatus::Pending,
            is_mutating: true,
            external_idempotency_key: None,
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        let parsed: ToolEffectPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.status, ToolEffectStatus::Pending);
        assert!(parsed.tool_output.is_none());
        assert!(parsed.external_idempotency_key.is_none());
    }

    /// Enum variants serialize to snake_case strings.
    #[test]
    fn sender_type_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&SenderType::User).unwrap(),
            "\"user\""
        );
        assert_eq!(
            serde_json::to_string(&SenderType::Agent).unwrap(),
            "\"agent\""
        );
        assert_eq!(
            serde_json::to_string(&SenderType::System).unwrap(),
            "\"system\""
        );
    }

    #[test]
    fn conversation_event_type_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&ConversationEventType::Message).unwrap(),
            "\"message\""
        );
        assert_eq!(
            serde_json::to_string(&ConversationEventType::Reply).unwrap(),
            "\"reply\""
        );
        assert_eq!(
            serde_json::to_string(&ConversationEventType::Reaction).unwrap(),
            "\"reaction\""
        );
        assert_eq!(
            serde_json::to_string(&ConversationEventType::Edit).unwrap(),
            "\"edit\""
        );
        assert_eq!(
            serde_json::to_string(&ConversationEventType::System).unwrap(),
            "\"system\""
        );
    }

    #[test]
    fn route_decision_outcome_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&RouteDecisionOutcome::Trigger).unwrap(),
            "\"trigger\""
        );
        assert_eq!(
            serde_json::to_string(&RouteDecisionOutcome::Skip).unwrap(),
            "\"skip\""
        );
        assert_eq!(
            serde_json::to_string(&RouteDecisionOutcome::Error).unwrap(),
            "\"error\""
        );
    }

    #[test]
    fn agent_result_status_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&AgentResultStatus::Succeeded).unwrap(),
            "\"succeeded\""
        );
        assert_eq!(
            serde_json::to_string(&AgentResultStatus::Failed).unwrap(),
            "\"failed\""
        );
    }

    #[test]
    fn tool_effect_status_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&ToolEffectStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&ToolEffectStatus::Executing).unwrap(),
            "\"executing\""
        );
        assert_eq!(
            serde_json::to_string(&ToolEffectStatus::Succeeded).unwrap(),
            "\"succeeded\""
        );
        assert_eq!(
            serde_json::to_string(&ToolEffectStatus::Failed).unwrap(),
            "\"failed\""
        );
    }

    #[test]
    fn dead_letter_source_type_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&DeadLetterSourceType::Command).unwrap(),
            "\"command\""
        );
        assert_eq!(
            serde_json::to_string(&DeadLetterSourceType::ToolEffect).unwrap(),
            "\"tool_effect\""
        );
        assert_eq!(
            serde_json::to_string(&DeadLetterSourceType::Delivery).unwrap(),
            "\"delivery\""
        );
    }

    /// Command priority serializes to its string variant name, not the i32 repr.
    #[test]
    fn command_priority_serializes_as_string() {
        assert_eq!(
            serde_json::to_string(&CommandPriority::Normal).unwrap(),
            "\"Normal\""
        );
        assert_eq!(
            serde_json::to_string(&CommandPriority::High).unwrap(),
            "\"High\""
        );
        assert_eq!(
            serde_json::to_string(&CommandPriority::Urgent).unwrap(),
            "\"Urgent\""
        );
    }

    /// Route decision with "trigger" decision and all fields populated.
    #[test]
    fn route_decision_trigger_variant() {
        let payload = RouteDecisionPayload {
            route_id: RouteId::new(),
            message_id: MessageId::new(),
            agent_id: "agent-reviewer".into(),
            conversation_id: "conv-5".into(),
            decision: RouteDecisionOutcome::Trigger,
            reason: "mentioned".into(),
            policy_snapshot: serde_json::json!({
                "mode": "mentioned_only",
                "aliases": ["reviewer", "rev"]
            }),
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        let parsed: RouteDecisionPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.decision, RouteDecisionOutcome::Trigger);
        assert_eq!(parsed.reason, "mentioned");
    }

    /// Envelope can carry a nested ConversationEventPayload in its payload
    /// field, and the consumer can deserialize it.
    #[test]
    fn envelope_carries_typed_payload() {
        use super::super::envelope::EventEnvelope;

        let inner = ConversationEventPayload {
            message_id: MessageId::new(),
            conversation_id: "conv-1".into(),
            seq: 1,
            sender_id: "user-1".into(),
            sender_type: SenderType::User,
            content: Some("hello".into()),
            content_type: "text".into(),
            event_type: ConversationEventType::Message,
            client_msg_id: Some("cmid-1".into()),
            turn_id: None,
            reply_event_id: None,
            metadata: serde_json::json!({}),
        };

        let inner_json = serde_json::to_value(&inner).expect("inner serialize");
        let envelope = EventEnvelope::new(
            inner.message_id.to_string(),
            "message",
            "conversation_event",
            &inner.conversation_id,
            inner_json,
            "trace-abc",
            "org-1",
        );

        // Serialize envelope, then deserialize and extract typed payload
        let wire = serde_json::to_string(&envelope).expect("envelope serialize");
        let parsed_envelope: EventEnvelope =
            serde_json::from_str(&wire).expect("envelope deserialize");
        let extracted: ConversationEventPayload =
            serde_json::from_value(parsed_envelope.payload).expect("payload extract");

        assert_eq!(extracted.message_id, inner.message_id);
        assert_eq!(extracted.seq, 1);
    }

    /// Dead letter with ToolEffect source type.
    #[test]
    fn dead_letter_tool_effect_variant() {
        let payload = DeadLetterPayload {
            source_type: DeadLetterSourceType::ToolEffect,
            source_id: "tc-failed-1".into(),
            payload: serde_json::json!({"tool": "webhook", "url": "https://example.com"}),
            error: "connection refused".into(),
            attempt_count: 3,
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        let parsed: DeadLetterPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.source_type, DeadLetterSourceType::ToolEffect);
    }

    /// Reject unknown enum variant from JSON.
    #[test]
    fn unknown_sender_type_is_rejected() {
        let json = r#""robot""#;
        let result: Result<SenderType, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    /// Reject unknown event type variant from JSON.
    #[test]
    fn unknown_event_type_is_rejected() {
        let json = r#""deleted""#;
        let result: Result<ConversationEventType, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    /// AgentCommandPayload with Urgent priority.
    #[test]
    fn agent_command_urgent_priority() {
        let payload = AgentCommandPayload {
            command_id: CommandId::new(),
            route_id: RouteId::new(),
            session_key: "agent-1:conv-1".into(),
            agent_id: "agent-1".into(),
            conversation_id: "conv-1".into(),
            message_id: MessageId::new(),
            turn_id: TurnId::new(),
            prompt: "URGENT: production is down".into(),
            priority: CommandPriority::Urgent,
            metadata: serde_json::json!({}),
        };

        let json = serde_json::to_string(&payload).expect("serialize");
        let parsed: AgentCommandPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.priority, CommandPriority::Urgent);
    }
}
