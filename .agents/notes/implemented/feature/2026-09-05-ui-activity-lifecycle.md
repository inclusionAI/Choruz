# Agent Note: UI actions and HTTP outcomes share activity traces

Status: implemented

## Problem

Recording a completed operation alone cannot distinguish a click that never
started work from a request that failed. Recording only successful sends also
hides the failed attempts that explain a user's retry.

## Decision

`choruz-trace.ts` emits separate started and finished events with one span ID
and distinct event IDs. The finished event records an outcome and duration.
`transportFetch` observes requests through both local and relay transports,
propagates the trace header, and preserves the original response or exception.
Telemetry delivery uses its captured transport directly, avoiding recursion.

Authenticated dashboard listeners record actionable clicks, selections, toggles,
file counts, form submissions, selected command keys, copy/paste intent, settled
scroll position and page visibility. The [component input contract](2026-09-06-component-input-activity.md)
owns bounded input snapshots and sensitive-field exclusions. Clipboard
contents, file bytes and raw keystrokes are excluded. Dynamic message, attachment and folder
labels are omitted; stable control attributes identify those actions. A
`data-activity-private` ancestor excludes a control from UI capture.

`ChatApp` supplies current Company and conversation context, plus the selected
terminal binding's device and account IDs when present. A span retains the
context and authenticated queue where it started, even if the view changes.
Listeners are removed at unmount; scroll timers do not outlive their owner.

## Alternatives considered

**Instrument every API caller separately.** That duplicates failure and timing
handling and lets new callers silently omit coverage. The shared transport is
the existing local/remote boundary.

**Record DOM text, form values and clipboard bytes.** These can expose drafts,
filenames or credentials. Control identity and explicit committed interaction
records serve different purposes and must not be conflated.

## Consequences

HTTP success describes the request, not user satisfaction or completion of a
long-running AI turn. A missing finish event is not proof of abandonment.
Trace association at UI entry is a correlation aid, not a causal proof for all
background work. Terminal byte streams are not captured by these DOM listeners.
Page-close capture remains best effort until local storage
commits, as described by the [durable activity owner](../architecture/2026-09-05-durable-browser-activity.md).

## Testing

`telemetry.spec.ts` exercises send failure, draft retention and successful retry
through real UI and database persistence; the first HTTP response is fault
injected. It checks matching request headers, start/finish pairs, context and
separate draft observations. Import checkbox actions verify control identity and both
values. Transport tests cover HTTP failure, cancellation and untouched bodies.
Client-side navigation remounts the dashboard in the same document before a
clipboard event; database assertions check single delivery and omitted contents.
