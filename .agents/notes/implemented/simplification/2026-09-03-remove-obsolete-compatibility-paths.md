# Agent Note: Reject obsolete compatibility inputs instead of guessing

Status: implemented

## Problem

Several pre-release compatibility paths silently accepted data that Choruz does not use: fanout query cursors duplicated the durable cursor store, `/v1/console` emitted an empty `presences` object for a removed table, `/v2/ingest` ignored a caller-supplied `sender_id`, an unknown agent driver silently selected `CLAUDE.md`, empty webhook secrets produced an `unsigned` marker, and one-line process records bypassed the PID start-time fence. These paths hide configuration and client defects behind superficially successful requests.

## Decision

Each contract has one accepted shape. `/ws/fanout` accepts only `user_id` and `client_id`; `/v1/console` omits presence data; `/v2/ingest` rejects unknown fields and derives the sender solely from authentication; instruction bootstrapping refuses an unknown driver; webhook delivery refuses an empty secret before making a request; and host lifecycle ownership requires both PID and recorded process start time.

The active `metadata.workflow` task-routing contract, unread counters, quote-reply semantics, company-less workspace authorization and top-level event `sender_id` remain because they carry current product behaviour rather than unused compatibility data.

## Alternatives considered

**Keep accepting and logging obsolete inputs.** This appears friendly to stale callers but preserves multiple public contracts and lets a warning be missed while the wrong behaviour continues.

**Remove every field or branch described as legacy in one sweep.** Several such paths still implement active product semantics. Classification by actual readers and tests avoids deleting current behaviour merely because its comment uses historical language.

**Add feature flags for strict validation.** Choruz has no external users and its repository rules require root-cause changes without transition flags.

## Consequences

Stale callers fail where they cross the contract instead of receiving an empty or guessed result. Operators must recreate a complete process record and configure a real webhook secret. The code and documentation have fewer modes, while active routing, authorization and message behaviour stay unchanged.

## Testing

Unit and browser tests assert that removed fanout fields, ingest `sender_id`, unknown drivers, empty webhook secrets and one-line process records are rejected, and that the console response omits `presences`.
