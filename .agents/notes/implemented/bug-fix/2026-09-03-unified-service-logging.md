# Agent Note: Use one strict logging configuration for every Rust process

Status: implemented

## Problem

Choruz Rust processes initialized `tracing` independently. Only the API Gateway supported JSON output, the replay CLI used a different default level, and invalid filters silently selected a default. Operators could not apply one logging configuration to the stack or distinguish configuration mistakes from service behaviour.

## Decision

Every Rust entry point calls `choruz_infrastructure::init_tracing`. `RUST_LOG` selects the tracing filter and defaults to `info`; `CHORUZ_LOG_FORMAT` accepts `human` or `json` and defaults to `human`. Invalid values fail process startup. Logs go to stderr so command output on stdout remains machine-readable.

Agent runtime binding lookup preserves database errors separately from an absent binding. Neither case claims that execution falls back to another workspace or driver.

## Alternatives considered

**Keep per-process tracing initialization.** This permits different defaults but makes the same environment variables mean different things across one local stack.

**Silently replace invalid values with defaults.** This keeps processes running but hides operator mistakes and makes an unexpected logging mode difficult to diagnose.

**Add a separate verbose flag.** `RUST_LOG` already expresses global and module-specific verbosity, so another switch would create overlapping modes.

## Consequences

One environment configuration controls every Rust process and malformed values are visible at startup. The replay CLI emits `info` by default like the services. JSON logs and human-readable logs carry the same events, and sensitive Remote Control values remain excluded at the event sites.
