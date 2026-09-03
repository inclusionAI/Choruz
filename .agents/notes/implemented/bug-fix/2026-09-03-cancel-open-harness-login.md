# Agent Note: Cancel an open harness login instead of waiting out its expiry

Status: implemented

## Problem

The sign-in panel's Cancel button only closed the panel. The `harness_account_login` row stayed open, so `harness_account_login_open_account_idx` refused a new sign-in for that account for up to 15 minutes with "this account already has a login in progress". The `cancelled` state existed in the schema, the docs and the web type, but nothing wrote it, and the gateway's in-process Claude driver kept polling for a callback the user was never going to paste.

## Decision

`POST /v1/companies/{c}/harness-accounts/{id}/logins/{login_id}/cancel` moves an open login (`queued`, `awaiting_browser`, `authorizing`) to `cancelled`, clears the link, device code and callback, and answers 204; a login that is not open answers 409. The web proxy `POST /api/harness-accounts/{id}/login/{login_id}/cancel` forwards it, and the panel's Cancel button calls it before closing when a login is still open. `DbLoginSink::take_callback` now reports an error once the row is no longer open, so the gateway's Claude driver stops at its next one-second poll; `fail_login` only touches open rows, so the cancelled row and the account keep their state.

## Alternatives considered

- **Expire the old row when a new login starts**: rejected. It hides the user's intent (the old sign-in may still complete in another tab) and leaves the driver running until it times out.
- **Abort the spawned task through a stored `JoinHandle`**: rejected. The database row is the shared state between the browser, the gateway and a connector; a cancelled state the driver reads works for all three without process-local bookkeeping.

## Consequences

- A Codex login on the gateway waits on the app-server's completion notification rather than polling, so a cancelled Codex sign-in ends when the device-code flow times out; the row is already `cancelled`, so nothing else observes the wait.
- A connector polls `callback/claim`, which keeps answering "no code" for a cancelled login until the connector's own timeout; its `fail` call then no-ops.
