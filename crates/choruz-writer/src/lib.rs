//! Conversation Writer for the Choruz message pipeline.
//!
//! This crate provides:
//!
//! - **Result consumption** (G1) — consume agent results from a channel
//! - **Turn deduplication** (G2) — check turn_id uniqueness before committing
//! - **Reply event writing** (G3) — write reply events to conversation_events
//!   with the turn_id UNIQUE constraint as the final dedup barrier
//! - **Writer loop** (G4) — continuous loop consuming from an mpsc channel
//!
//! # Architecture
//!
//! The writer receives `AgentResult` values (from the executor via a channel),
//! filters to succeeded results, and commits them as reply events.  The
//! turn_id UNIQUE constraint on conversation_events guarantees that at most
//! one reply is committed per logical turn, even if multiple attempts succeed.

pub mod models;
pub mod writer;

// Re-export commonly used types at crate root.
pub use models::{AgentResult, AgentResultStatus, CommandAttemptRef, WriteOutcome};
pub use writer::{
    InMemoryResultStore, ResultStore, WriterConfig, WriterError, WriterResult, commit_result,
    run_writer_loop,
};
