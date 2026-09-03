//! Session Manager crate for the Choruz message pipeline.
//!
//! This crate provides:
//!
//! - **Session registry CRUD** — create, read, update sessions
//!   (`session_registry` table)
//! - **Command state machine** — insert, query, transition agent commands
//!   (`agent_commands` table)
//! - **Lease management** — assign leases with epoch bumps, validate epochs,
//!   release leases
//! - **Heartbeat monitoring** — detect expired leases, trigger retry or
//!   dead-letter
//! - **Retry scheduling** — exponential backoff (2^n seconds, max 300s),
//!   max 3 attempts
//! - **Dead letter management** — write and list unprocessable commands
//! - **Executor registry** — in-memory registry of available executor nodes
//!
//! # Architecture
//!
//! The `PgSessionStore` owns a `deadpool_postgres::Pool` and implements all
//! database operations.  The executor registry is kept in-memory (behind an
//! `Arc<RwLock<HashMap>>`) because executor nodes are ephemeral and register
//! via heartbeats.

pub mod error;
pub mod models;
pub mod retry;
pub mod store;

// Re-export the most commonly used types at crate root.
pub use error::{SessionError, SessionResult};
pub use models::*;
pub use retry::{
    DEFAULT_MAX_ATTEMPTS, MAX_BACKOFF_SECS, exponential_backoff_secs, is_exhausted, next_retry_at,
};
pub use store::PgSessionStore;
