# Agent Note: Record semantic component inputs through the activity outbox

Status: implemented

## Problem

Click counts cannot explain which selection or draft revision led to an action. CSS classes and generated element IDs do not reliably identify the component across renders. The [UI activity lifecycle](2026-09-05-ui-activity-lifecycle.md) owns event correlation, but its control-only projection does not meet component-input analysis needs.

## Decision

The authenticated dashboard observes component input snapshots and semantic actions through its existing tracing and durable outbox. Explicit `data-activity` identifiers take precedence over accessible labels; the shared modal supplies a surface name. Inputs record each changed snapshot, including clearing, and suppress the duplicate native change event after input. Selections retain their value and index. Dialog open and close observations cover cancellation without asserting why a dialog closed.

Snapshots contain at most 2048 characters with length and truncation metadata. Password, authentication, invitation and explicitly private fields emit an omission marker without a value or length. Filesystem values, clipboard bytes, file contents and raw terminal input are not collected by this listener. Known secret markers in ordinary text are redacted; this cannot recognize arbitrary secrets pasted into a general-purpose message field. Operators must treat retained drafts as private user data with the same actor-scoped access and retention controls as the existing activity store.

## Alternatives considered

**Keep only clicks and committed messages.** This loses edits, abandoned input and selected options, which are the requested analysis surface.

**Add a second recorder to every handler.** This duplicates the browser and remote delivery lifecycle and invites double counting. Delegated listeners cover native controls; special controls use explicit semantic attributes.

**Record coordinates, raw keys or unlimited values.** Those are not component semantics and would capture credentials or unbounded documents. Bounded snapshots make the exclusion and truncation visible to consumers.

## Consequences

The durable outbox and gateway remain the sole persistence path. Browser storage failure can still lose observations, and a UI event is not proof that an operation completed. Custom canvas interactions require an explicit semantic event; this is not a recording of every pixel interaction or a native CLI transcript. UI lifecycle listeners are removed on unmount and do not create an additional polling loop. The HTTP lifecycle and committed-message projection retain their independent contracts.

## Testing

`activity-control.test.ts` checks value capture, clearing, bounds and sensitive-field exclusion. `telemetry.spec.ts` drives actual draft edits, cancellation, selection and a password field, then reads actor-scoped rows from PostgreSQL. Its failed-send scenario distinguishes draft observations from committed-message persistence.
