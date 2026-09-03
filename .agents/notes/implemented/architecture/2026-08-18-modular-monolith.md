# Agent Note: Start with a modular monolith

Status: implemented

Formerly ADR-001.

## Problem

The product needs direct chat, group chat, native agents, and a browser UI in v1, but the repository started from zero. Splitting into microservices at that point would have multiplied operational work before core product behaviour was validated.

## Decision

Choruz is a modular monolith in Rust. Boundaries stay explicit through separate crates: `crates/choruz-domain` (entities and invariants), `crates/choruz-application` (the `DbService` business logic), `crates/choruz-infrastructure` (PostgreSQL and I/O), and `crates/choruz-auth`. `services/choruz-api-gateway` and the message pipeline under `services/choruz-pipeline` compose those crates; the realtime feed, the agent-facing routes and background scheduling live inside those two processes, and any of them can be split out later without rewriting the core model. The split points are recorded here, not held open by placeholder binaries. `docs/architecture.md` is the current map.

<!-- agent-note-format: alternatives-not-recorded (pre-format Agent Note) -->

## Consequences

- End-to-end chat behaviour shipped quickly because one process could own the whole flow.
- The future split points are named (API gateway, realtime gateway, agent gateway, job runner) but not scaffolded: a split starts by moving a module out of `choruz-api-gateway` or the pipeline, not by reviving an empty binary.
- Operational load stays low: one PostgreSQL, a handful of binaries, no service mesh.
