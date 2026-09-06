# Agent Note: Browser activity is acknowledged after database commit

Status: implemented

## Problem

Clearing a browser queue before the server confirms persistence loses activity
on ordinary network failures. Retrying without stable identity instead inflates
counts when the response, rather than the request, is lost.

## Decision

`apps/web/lib/api/activity-outbox.ts` persists sanitized events in IndexedDB.
The authenticated gateway and principal identify each queue. Delivery captures
the current transport at trace initialization, so switching a remote dashboard
cannot redirect a previous device's pending batch. Tokens remain in memory.

`DbService::record_telemetry` owns batch validation and one database transaction.
`telemetry_event` deduplicates by authenticated workspace, principal and event ID.
Client occurrence time and server receipt time remain distinct. HTTP 204 means
the entire batch committed; only then does the browser remove those IDs.
Failures retain the batch, with retry intervals increasing to one minute.

The log-only analytics route has no independent owner; existing callers emit
through `choruz-trace.ts`. Browser events are client-reported observations, not
authoritative security audit records or proof that an operation succeeded.

## Alternatives considered

**Keep an in-memory queue.** It cannot survive refresh or tab closure.

**Clear on request dispatch.** A rejected request would permanently lose events.

**Create a separate analytics database.** The existing telemetry table already
owns these records; a second store would duplicate identity and access rules.

## Consequences

Replay has one persisted effect, but does not promise capture before IndexedDB
accepts an event. Browser storage denial, eviction and quota exhaustion can
still lose observations and emit a warning. Oversized data is replaced by an
omission marker, preserving event identity without blocking every later event.
This does not add full UI coverage, terminal transcripts or analytical reports.

Existing rows have nullable new fields; their missing occurrence time cannot be
reconstructed from receipt time. New clients require the versioned ingest
contract. A coordinated web/gateway deployment is required.

## Testing

`tests/e2e/telemetry.spec.ts` loses a real commit acknowledgement, reloads the
browser and checks one database row and an emptied queue. Gateway observability
tests force a second insert failure and verify rollback, invalid batch refusal,
actor-scoped replay and redaction. Outbox tests cover reopen, actor isolation
and concurrent append during acknowledgement.

## Related

The [remote dashboard transport](2026-09-03-remote-dashboard-transport.md)
continues to own encrypted delivery, not activity persistence.
