# Agent Note: Contract-first external APIs

Status: implemented
Archived: 2026-09-03

Formerly ADR-002.

## Problem

Human clients, agents, SDKs, and local operations tooling all depend on stable contracts. Breaking wire formats later would slow every client implementation.

## Decision

Protobuf (`proto/choruz/v1/chat.proto`, linted by `buf`) and OpenAPI (`openapi/choruz.yaml`) are product artifacts, frozen before deeper implementation rather than generated after the fact. The SDKs under `sdk/` (Rust, TypeScript, Python) follow the contracts; `pnpm contracts:check` and the SDK checks run in CI's Static checks job when those paths change.

<!-- agent-note-format: alternatives-not-recorded (pre-format Agent Note) -->

## Consequences

- One contract source serves REST and agent workflows.
- SDK generation and name consistency (`scripts/check-choruz-sdk-names.sh`) have a single reference.
- Spec gaps surface early, from the test plan, instead of after clients ship.
