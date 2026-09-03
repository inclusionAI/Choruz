# Agent Note: Workspace-scoped isolation from day one

Status: implemented

Formerly ADR-003.

## Problem

Multi-tenancy was a later-phase goal, but tenant leaks are expensive to remove after the fact. The core data model had to carry workspace boundaries before any tenant existed.

## Decision

`workspace_id` is attached to principals, conversations, messages, and audit records. Every command in `crates/choruz-application` enforces same-workspace access, and the e2e suite (`apps/web/tests/e2e/workspace-isolation.spec.ts`) pins the isolation rules.

<!-- agent-note-format: alternatives-not-recorded (pre-format Agent Note) -->

## Consequences

- A multi-tenant rollout is an exposure problem, not a data model rewrite.
- Isolation is testable now, per command, rather than audited later.
