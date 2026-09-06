//! Shared host-side plumbing for Choruz:
//!
//! - [`connectors::ConnectorSupervisor`] owns persistent runtime connectors.
//! - [`pg::EmbeddedPg`] spins up a private PostgreSQL + applies migrations.
//! - [`supervisor::Supervisor`] spawns `choruz-api-gateway` / `choruz-pipeline` as
//!   child processes, with a converging shutdown path (Drop + explicit
//!   `shutdown()`) so they die with the parent.
//!
//! `choruz-server` consumes the PostgreSQL and backend pieces for headless mode.
//! `choruz-api-gateway` owns the connector supervisor in every deployment layout.

pub mod connectors;
pub mod pg;
pub mod supervisor;
