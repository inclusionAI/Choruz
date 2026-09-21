//! Evaluation and optimization without a database, runtime or model provider.
//!
//! The caller executes search actions and persists checkpoints. Selection is
//! evidence only: this crate never installs guidance or changes a running agent.

pub mod evaluation;
pub mod optimization;
pub mod team;
