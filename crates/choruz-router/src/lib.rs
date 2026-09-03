//! Router / Policy Engine for the Choruz message pipeline.
//!
//! This crate provides:
//!
//! - **Event consumption** (C1) — parse outbox events from the CDC poller channel
//! - **Mailbox visibility** (C2) — write visibility records for all agent members
//! - **Policy evaluation** (C3) — MentionedOnly / AllMessages / Manual trigger policies
//! - **Route decisions** (C4) — always write audit records (trigger, skip, or suppress)
//! - **Agent command generation** (C5) — create commands for triggered agents
//!
//! # Architecture
//!
//! The router consumes `OutboxRow` entries from the in-memory CDC channel
//! (provided by `choruz-store::CdcPoller`), looks up conversation membership
//! via a `MemberProvider`, evaluates trigger policies, and writes outputs
//! via a `DecisionSink`.
//!
//! Both traits are pluggable: the production implementation uses PostgreSQL
//! while tests use in-memory implementations.

pub mod models;
pub mod policy;
pub mod router;
pub mod workflow;

// Re-export commonly used types at crate root.
pub use models::{
    AgentPolicy, AssignedTaskHint, AssigneeRosterEntry, AutoMode, ChannelTaskStatus,
    ConversationMember, ConversationRoutingPolicy, GroupWorkflowTask, GroupWorkflowTaskParticipant,
    MailboxVisibility, RouteDecision, RouteOutcome, TriggerResult, UntaggedHumanMode,
    WorkflowRoutingEvent,
};
pub use policy::evaluate_trigger;
pub use router::{
    DecisionSink, InMemoryDecisionSink, InMemoryMemberProvider, MemberProvider, RouterConfig,
    RouterError, RouterResult, route_event, run_router_loop,
};
pub use workflow::parse_workflow_routing_event;
