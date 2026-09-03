//! Executor crate for the Choruz message pipeline.
//!
//! Provides CLI adapters (Claude Code, Codex), sandbox management,
//! local WAL for crash recovery, and shared types.

pub mod adapter;
pub mod codex_adapter;
pub mod error;
pub mod sandbox;
pub mod wal;
