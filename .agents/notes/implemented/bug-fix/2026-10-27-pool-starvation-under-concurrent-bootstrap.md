# Agent Note: Never wait for a second pooled connection while holding one

Status: implemented

## Problem

`DbService::accessible_workspace_ids` held a pooled connection for the native workspace check and then called `list_companies`, which acquires its own. `/v1/bootstrap` reaches it through runtime bindings while its other lookups also hold connections. Under 100 concurrent bootstraps every slot belonged to a request waiting for another slot; the pool has no wait timeout, so about half the requests stalled until the client gave up at sixty seconds. The performance smoke that exercises this load ran only on pushes to `main`, so the pull request merged green and `main` turned red afterwards.

## Decision

A database owner releases its connection before calling another owner that acquires one; `accessible_workspace_ids` scopes the native check's connection. `EventStore::with_max_connections` builds a pool of an explicit size, and a gateway test runs `/v1/bootstrap` against a single-connection pool, where any nested acquisition on that path hangs instead of passing.

The performance smoke is required on every pull request that triggers DB and API smoke (server, schema, host-lifecycle or workflow changes), not only on `main`. The [PR test policy](../../../../docs/testing/pr-test-policy.md#what-ci-runs-for-which-paths) lists it with those paths.

## Alternatives considered

**Raise the pool size or add an acquisition timeout.** A larger pool moves the threshold; a timeout turns the stall into errors. Neither removes the hold-and-wait.

**Pass the held client into `list_companies`.** That shares the query but widens a public signature for one caller; releasing the connection is local and keeps one owner for company listing.

## Consequences

The single-connection test covers the whole bootstrap path, so a later nested acquisition there fails deterministically rather than only under CI load. Other request paths are not covered by that test; the performance smoke remains their load-level guard. Pull requests touching the server spend one release build and a short k6 run on the extra job.
