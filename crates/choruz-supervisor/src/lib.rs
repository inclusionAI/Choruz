//! Shared host-side plumbing for Choruz:
//!
//! - [`pg::EmbeddedPg`] spins up a private PostgreSQL + applies migrations.
//! - [`supervisor::Supervisor`] spawns `choruz-api-gateway` / `choruz-pipeline` as
//!   child processes, with a converging shutdown path (Drop + explicit
//!   `shutdown()`) so they die with the parent.
//!
//! `choruz-server` consumes this crate: headless mode used over SSH tunnels,
//! Postgres + backend services with no frontend. Clients connect via tunnel
//! to the remote gateway port.

pub mod pg;
pub mod supervisor;
