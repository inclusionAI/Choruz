# Agent Note: Share the activity contract without platform storage

Status: implemented

## Problem

Independent capability consumers need the same event validation and defensive
redaction as the platform. Keeping those pure operations inside the database
adapter and HTTP handlers forces a server dependency or a copied implementation.

## Decision

`choruz-activity` owns `TelemetryEvent`, batch validation and recursive/text
redaction. The application database adapter and gateway call this shared owner;
there is no forwarding implementation in either old location. The library has
no PostgreSQL, PTY, scheduler or HTTP server dependency.

HTTP ingest sanitizes before validation. Persistence still validates before an
atomic, actor-scoped transaction. Export and background evidence screening use
the same redactor. Browser filtering remains before IndexedDB storage, where
server-side filtering cannot protect a queued payload.

The [durable browser activity decision](2026-09-05-durable-browser-activity.md)
retains acknowledgement, deduplication and recovery ownership. The
[activity tools decision](../feature/2026-09-05-activity-data-tools.md) retains
authenticated export and explicit retention ownership. Neither is superseded
by a storage-free library.

## Alternatives considered

**Expose gateway helpers.** Consumers would inherit the HTTP server and database
composition just to prepare a record.

**Add another collector and store.** That would duplicate identity, delivery and
retention semantics rather than decouple the existing capability.

## Consequences

Independent consumers can prepare events without starting Choruz. They must
still supply consent, authenticated context, transport and durable storage.
Pattern redaction does not certify arbitrary text as safe for publication.
Native trace collection remains a separate private evidence path.

Library tests pin the wire shape, bounds, recursive redaction and ordering;
existing gateway tests retain atomicity, actor isolation and export coverage.
The standalone example demonstrates preparation only, not a running collector.
