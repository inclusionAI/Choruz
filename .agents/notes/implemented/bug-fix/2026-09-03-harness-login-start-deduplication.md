# Agent Note: Deduplicate harness login starts in the browser

Status: implemented

## Problem

The Harness account panel can mount its sign-in subpanel more than once while the surrounding account list updates. Each mount started `POST /api/harness-accounts/{id}/login`, even though the gateway permits only one open login for an account. The second request therefore returned a structured conflict body, but the UI rendered that object as `[object Object]` instead of the actionable detail.

## Decision

`apps/web/components/agents/harness-account-picker.tsx` shares an in-flight start request by company and account for a short post-settlement window. A remount receives the same `HarnessLogin` response instead of creating a duplicate login. The panel reads both string errors and the gateway's `{ error: { detail } }` form for start, poll, and callback failures, and identifies the selected Harness in its heading. [One sign-in flow for harness accounts on any device](../feature/2026-09-03-local-harness-login-handoff.md) remains the owner of the cross-device login protocol.

## Alternatives considered

- **Treat a duplicate-start conflict as success by fetching an existing login**: rejected. The browser does not have an endpoint that identifies the existing open login, and adding one would broaden the account-login contract to compensate for a local remount.
- **Keep issuing independent requests and hide the conflict**: rejected. It still makes an avoidable state-changing call and leaves a race whose result depends on timing.
- **Render unknown error values with `String(error)`**: rejected. The API's structured detail is the user-facing diagnostic; a generic coercion hides it.

## Consequences

- A quick re-render does not create a second login or show a false "already in progress" error.
- The one-second entry lifetime intentionally covers React remount churn without caching a completed login for an extended period; a later user retry starts a fresh request.
- `apps/web/tests/e2e/modals.spec.ts` asserts one start request for the normal flow and readable text for a structured conflict.
