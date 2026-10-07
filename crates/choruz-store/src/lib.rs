//! Conversation event store and outbox for the Choruz durable message pipeline.
//!
//! This crate owns all database operations for:
//! - `conversation_events` — append-only event log per conversation
//! - `event_outbox` — CDC source for the durable bus
//! - CDC poller — polls outbox and dispatches to an in-memory channel

#[cfg(feature = "postgres")]
pub mod cdc_poller;
pub mod conversation_events;
pub mod event_outbox;
#[cfg(feature = "postgres")]
mod pool;

#[cfg(feature = "postgres")]
pub use cdc_poller::{CdcPoller, CdcPollerConfig, CdcPollerHandle};
pub use conversation_events::{ConversationEvent, ConversationEventRow, ThreadFlags};
pub use event_outbox::{OutboxEntry, OutboxRow};
#[cfg(feature = "postgres")]
pub use pool::EventStore;
