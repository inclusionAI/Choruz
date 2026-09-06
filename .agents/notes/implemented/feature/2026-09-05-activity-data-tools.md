# Agent Note: Activity tools use authenticated reads

Status: implemented

## Problem

Durable observations need a usable export and an explicit retention operation. Direct database access would give the CLI different permissions from the dashboard. Copying messages into telemetry would create another content owner.

## Decision

The activity API filters by authenticated actor and their currently accessible workspaces. Listing and aggregation share the same source and time filters. The CLI follows keyset pages through that API and exports committed conversation content only through the membership-gated interaction endpoint. Audit projections exclude arbitrary historical metadata; telemetry is sanitized again before export.

Retention is an explicit, bounded telemetry-only operation. Preview is the default, deletion requires apply, and the deletion audit marker participates in the same transaction. Canonical conversations and audit history are not part of analytics cleanup. Metrics use the existing shared registry and bounded outcome/status labels.

## Alternatives considered

**Give the analytics CLI direct database access.** That would bypass the shared HTTP permissions and content-read audit trail.

**Automatically delete old data on startup.** A deployment is not a retention decision. Explicit cutoff and apply let operators set their policy without losing history unexpectedly.

**Interpret absent events as abandonment.** Browser storage loss and disconnected processes can omit events. Summaries only count observed events and explicit outcomes.

## Consequences

Data export is per API host and signed-in human, not a central cross-device warehouse or a multi-user administrator report. Server receipt time bounds queries; client occurrence time remains a separate observation. Long intervals require multiple bounded exports. Native CLI text stays in the Harness, and terminal transport metadata cannot reconstruct it. An interrupted JSON-lines export can be partial and exits nonzero on a surfaced error. Export files need separate access and retention controls.

## Testing

Gateway tests exercise actor isolation, live company membership, pagination, explicit summary outcomes, redaction, validation and rollback when the deletion audit insert fails. The CLI E2E uses a compiled binary against the real API and database to export multiple pages, summarize them and opt into committed message content. No model provider is substituted or invoked by this data export test.
