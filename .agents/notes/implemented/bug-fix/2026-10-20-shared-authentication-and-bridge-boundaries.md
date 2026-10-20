# Agent Note: Authentication, host reuse and bridge links retain their owners

Status: implemented

## Problem

Account probes selected a different executable policy from Agent turns. CLI host
startup accepted any successful readiness response. Bridge Agent creation sent
Bearer requests to a cookie-authenticated web route on the gateway address, and
channel commands announced newly created groups even when an existing database
mapping rejected that link.

## Decision

Harness authentication delegates executable selection to
`choruz_agent_runtime::executable::terminal_binary`. The CLI and supervisor both
validate `HostServiceStatus` through `validate_ready`; a reachable invalid listener
is an error, not an invitation to spawn another server.

The bridge keeps gateway authentication and the existing web provisioning owner.
Its required `choruz.web_url` identifies that deployment's web app. Only web
provisioning receives the session cookie; other API calls retain Bearer auth.
The route's `agent.id` is the returned principal identity. Exact driver IDs and an
empty default instruction satisfy the route's existing validation.

All channel creation enters `MappingStore::getOrCreate`. A PostgreSQL transaction
advisory lock keyed by platform and channel serializes the factory across bridge
processes. Existing mappings return their authoritative conversation without a
new factory call. Commands announce the returned ID, not a proposed group.

## Alternatives considered

**Permit Bearer authentication on all web routes.** Rejected because a bridge
adapter can satisfy the existing cookie boundary without weakening other callers.

**Upsert channel mappings after creating groups.** Rejected because concurrent
requests would still create unused groups and silently redirect later messages.
The canonical link is retained rather than introducing a new relink operation.

**Separate asynchronous readiness validation.** Rejected because HTTP transport
differs between callers, but service identity and protocol acceptance do not.

## Consequences

Bridge configurations must supply the web URL for the same deployment as the
gateway. Database-backed concurrency coverage observes lock contention and reads
the resulting mapping. HTTP-route coverage retains real cookie verification and
request validation while substituting gateway authentication and durable creation.
Native CLI fixtures prove executable/profile selection, not provider availability.

The group factory remains an HTTP operation outside the database transaction.
A storage or network failure after group creation can leave an unlinked group;
the bridge reports failure and does not claim that group is linked. This change
prevents competing factories during ordinary concurrent channel creation, not a
distributed transaction across the gateway and bridge database.
