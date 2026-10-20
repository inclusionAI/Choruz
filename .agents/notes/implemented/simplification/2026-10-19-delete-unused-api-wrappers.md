# Agent Note: Keep client API wrappers only for production consumers

Status: implemented

## Problem

Company management uses `apiFetch`, while unused typed company mutation and membership wrappers maintain a separate client surface tested only against mocked fetch. The single-binding read wrapper has the same ownership problem; runtime consumers use the binding list.

## Decision

`apps/web/lib/api/choruz-api.ts` exposes typed wrappers used by production consumers. Unused company collection, mutation and membership wrappers, their unused membership type, and the unused single-binding read wrapper are absent. Tests of shared request headers, methods and errors exercise live wrappers such as `fetchCompany`, `localLogin` and `revokeRuntimeHost`. Endpoint-specific tests that only sustain an unused wrapper are absent.

The HTTP routes and e2e fixture helpers retain their existing owners. Removing an unused browser export does not remove a server capability.

## Alternatives considered

**Keep wrappers for possible callers.** This creates a second maintained API surface without a product consumer. A typed wrapper is appropriate when an actual caller needs it.

**Remove all typed wrappers.** Existing production callers benefit from response types and a shared transport seam; deleting those wrappers would spread request construction instead of simplifying ownership.

## Consequences

Request-construction coverage remains attached to used functions. Company management and runtime-list callers keep their behavior, and new product callers must deliberately choose between the shared generic API helper and a typed wrapper.
