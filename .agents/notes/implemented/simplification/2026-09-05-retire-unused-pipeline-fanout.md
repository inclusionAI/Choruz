# Agent Note: Retire the unused pipeline fanout endpoint

Status: implemented

## Problem

The pipeline started a second realtime stack with its own event polling, connections and cursors even though repository product clients use the gateway sync feed. The only executable client reference was the web test runner's readiness probe, which accepted any HTTP response. The public endpoint's strict query contract was owned by the [compatibility-input decision](2026-09-03-remove-obsolete-compatibility-paths.md), but no product consumer justified retaining the endpoint.

## Decision

The pipeline serves health, readiness and metrics without a WebSocket fanout route or polling task. Its unused fanout crate and PostgreSQL event adapter are removed. The test runner checks the owned pipeline process through `/readyz`; the shared readiness helper requires HTTP 200 and the expected ready service identity.

The gateway sync feed remains the dashboard transport. Replay queries, channel-task events and historical migrations retain their owners. The removal retires a pre-release public endpoint; it does not claim external consumer discovery is exhaustive.

## Alternatives considered

**Retain the strict endpoint for unknown external clients.** Repository evidence identifies no product subscribers, and the authorized pre-release retirement avoids maintaining an unused protocol indefinitely. Its prior strictness rejected query cursors in favor of the cursor store; it was not evidence of active use.

**Delete the entire pipeline HTTP server.** Health, readiness and metrics are active operational contracts. Removing fanout does not remove those endpoints or fatal monitoring of their server task.

## Consequences

`/ws/fanout` returns 404. External callers of that retired endpoint must adopt the authenticated gateway sync protocol. No applied migration is edited, and the replay CLI retains its event-store query. Existing sync browser tests and an HTTP regression exercise the retained realtime and operational contracts; a shell regression rejects non-200 readiness responses even if their body claims readiness.
