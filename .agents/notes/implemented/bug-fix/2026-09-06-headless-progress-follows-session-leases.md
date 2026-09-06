# Agent Note: Headless progress follows session leases

Status: implemented

## Problem

Headless commands can run for minutes while their binding reports idle. A browser-local thinking timer cannot recover actual execution state after a refresh. Execution batches and lease recovery also make a single command's completion insufficient to declare the whole agent idle.

## Decision

Session lifecycle is the authority for headless binding progress. A database projection updates eligible idle/running bindings when a session enters or leaves active state, aggregating all sessions for the agent after acquiring its binding lock. Paused, disabled and error states retain their administrative meaning. Unchanged progress emits no extra binding update. The existing [binding sync path](../../feature/2026-09-03-runtime-bindings-in-the-sync-path.md) delivers changes and refresh snapshots.

Retry scheduling releases the session in the same attempt-fenced transaction when no active batch members remain. Result commit has one fenced release owner, with no later unfenced draining write. The successor migration reconciles existing leased work and inactive retry sessions. `in_flight_turn_id` is not synthesized: an agent's batch can own several turns, while running/idle is an aggregate state.

## Alternatives considered

- **Write state separately in each driver or executor:** rejected because local batch, remote claim, retry and expiry paths would become independent state machines.
- **Trigger on both commands and sessions:** rejected because command updates can precede session updates; taking the binding lock between them creates a binding/session lock inversion. Session-only projection preserves one lifecycle owner.
- **Use only the front-end thinking timer:** rejected because it cannot survive refresh or distinguish a long operation from a hung lease.

## Consequences

The change covers headless progress for every driver sharing the session store, without changing PTY byte transport or interpreting CLI output. A successful command remains running until its result is committed; retry backoff is idle. Real PostgreSQL regressions cover batch completion, retry, expiry, stale attempts, remote success/failure, protected states and list/detail/bootstrap/sync agreement. Runtime progress emits the existing sync changes; it is not a human administrative action in the audit log.
