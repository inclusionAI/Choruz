//! Tool Gateway crate for the Choruz message pipeline.
//!
//! Provides:
//!
//! - **Effect Journal** (`effect`) — domain types and CRUD for the
//!   `effect_journal` table, which tracks every external tool invocation
//!   for idempotent replay.
//!
//! - **Tool Registry** (`registry`) — static metadata about registered
//!   tools, classifying each as read-only or mutating with a specific
//!   [`registry::MutationPolicy`].
//!
//! - **Tool Gateway** (`gateway`) — the core idempotency layer that
//!   mediates all tool invocations through the effect journal.

pub mod effect;
pub mod gateway;
pub mod registry;
