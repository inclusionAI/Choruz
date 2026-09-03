//! Event envelope and topic payload definitions for the Choruz durable bus.
//!
//! This crate defines:
//!
//! - [`envelope::EventEnvelope`] — the unified wrapper for all bus messages
//! - [`topics`] — strongly-typed payload structs for each Kafka topic
//!
//! These types are shared between producers (Ingress API, Router, Executor,
//! Tool Gateway) and consumers (Router, Session Manager, Conversation Writer,
//! Fanout Gateway).

pub mod envelope;
pub mod topics;

// Re-export the most commonly used types at crate root for convenience.
pub use envelope::EventEnvelope;
pub use topics::{
    AgentCommandPayload, AgentResultPayload, AgentResultStatus, CommandPriority,
    ConversationEventPayload, ConversationEventType, DeadLetterPayload, DeadLetterSourceType,
    DeliveryPayload, RouteDecisionOutcome, RouteDecisionPayload, SenderType, ToolEffectPayload,
    ToolEffectStatus,
};
