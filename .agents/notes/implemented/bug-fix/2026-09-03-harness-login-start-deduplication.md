# Agent Note: Deduplicate harness login starts in the browser

Status: implemented

## Problem

The Harness account panel can mount its sign-in subpanel more than once while the surrounding account list updates. Each mount started `POST /api/harness-accounts/{id}/login`, even though the gateway permits only one open login for an account. The second request therefore returned a structured conflict body, but the UI rendered that object as `[object Object]` instead of the actionable detail.

## Decision

The browser-cache decision is superseded by [authoritative login recovery](2026-09-05-resume-open-harness-login.md). The independently useful error-rendering contract remains: the panel reads both string errors and the gateway's `{ error: { detail } }` form for start, poll, and callback failures, and identifies the selected Harness in its heading. [One sign-in flow for harness accounts on any device](../feature/2026-09-03-local-harness-login-handoff.md) remains the owner of the cross-device login protocol.

## Alternatives considered

- **Treat a duplicate-start conflict as success by fetching an existing login**: rejected. The browser does not have an endpoint that identifies the existing open login, and adding one would broaden the account-login contract to compensate for a local remount.
- **Keep issuing independent requests and hide the conflict**: rejected. It still makes an avoidable state-changing call and leaves a race whose result depends on timing.
- **Render unknown error values with `String(error)`**: rejected. The API's structured detail is the user-facing diagnostic; a generic coercion hides it.

## Consequences

- The short browser cache covered remount churn but could not recover an open task after later navigation; its replacement is owned by the linked recovery decision.
- `apps/web/tests/e2e/modals.spec.ts` retains readable text for structured errors and tests the official sign-in flow independently of request count.
