# Agent Note: One sign-in flow for harness accounts on any device

Status: implemented

## Problem

Adding a Harness account behaved differently by device. An account on a remote runtime host got the official browser flow: the connector ran the Harness login, the web panel showed the authorization link, the user pasted the callback and the account came back verified. An account on the gateway's own device got no link: an `isolated` profile showed a shell command (`CLAUDE_CONFIG_DIR=… claude /login`) to run by hand, and a `default` profile only probed whatever credentials already existed. The user's expectation, "pick the device, choose sign in, get a link", held only for remote hosts. The login driver also lived inside `services/choruz-connector/src/main.rs`, so nothing on the gateway side could reuse it, and the browser-side routes were scoped to a runtime host (`/v1/companies/{c}/runtime-hosts/{host}/harness-account-logins`), which a local account has no way to address.

## Decision

The login driver moves into `crates/choruz-harness-login` behind a `LoginSink` trait with two implementations: `HttpLoginSink` in the connector (the host-facing routes, unchanged on the wire) and `DbLoginSink` in the gateway (writes to `harness_account_login` directly). One login state machine serves both; the only difference is who claims the row. The browser-side routes are re-homed on the account (`POST /v1/companies/{c}/harness-accounts/{id}/logins`, `GET …/{login_id}`, `POST …/{login_id}/callback`) in the core router, and `start` reads the account's `runtime_host_id` to decide: a remote account is inserted `queued` for the connector; a local account is inserted already `authorizing` with `claimed_at` set, and the gateway spawns `run_local_login` in the same request. `V037` makes `harness_account_login.runtime_host_id` nullable for that case. The web picker shows the same `HarnessLoginPanel` for every account, the shell-command path and the "verify existing credentials" default for new accounts are gone, and a new account defaults to "Sign in to a new account" (`isolated`).

Claude's manual callback is the complete `authorization-code#state` value shown by its browser flow. The driver separates the value, validates the state against the authorization URL and passes the code and state as distinct control fields. Codex uses the app-server browser protocol on both local and remote devices. Local login completes through the browser's loopback redirect. A remote user opens the same link on the controlling computer, then pastes the complete localhost callback URL; the connector validates its state and forwards it only to the loopback listener named by the original authorization request. A Codex executable without `app-server` support fails with an update or `CHORUZ_CODEX_BINARY` instruction instead of leaving the account pending. The corrected browser callback, readiness, quota-label and removal behavior is owned by [Browser OAuth and fail-closed Harness account lifecycle](../bug-fix/2026-09-03-browser-oauth-account-readiness.md).

## Alternatives considered

- **Keep the shell command for local accounts**: rejected. It asks the user to leave the product, run a command with the right `CLAUDE_CONFIG_DIR`, and come back to press "Verify"; every step is one they cannot see from the panel, and the Codex equivalent needs a different command.
- **Run the local login inside the Next.js server instead of the gateway**: rejected. The web server already proxies every account mutation to the gateway and holds no Harness state; a second executor there would mean two places to keep the driver, and the connector could not share it.
- **A separate table or a separate route family for local logins**: rejected. The state machine, the TTL, the unique open-login index and the web polling are identical; only the executor differs, and a nullable `runtime_host_id` expresses that in one row.
- **Have the gateway claim local rows from a background poller like the connector does**: rejected. Claiming inside the start transaction gives the caller a row that is already running and avoids a poller that would only ever serve this process.

## Consequences

- One code path for the Harness login protocol; a protocol change lands in the crate and both executors follow.
- A gateway restart abandons an in-flight local login; the row expires after 15 minutes and the panel offers "Sign in" again. Acceptable before a persistent job runner exists.
- `CHORUZ_CLAUDE_BINARY` and `CHORUZ_CODEX_BINARY` now matter to the gateway as well as the connector; the gateway tests substitute protocol-faithful fake executables.
- A failed login marks a non-active account as `error` with a sanitized diagnostic, so the account list does not remain indefinitely `pending`.
- The host-scoped browser routes are deleted rather than kept as aliases, per the pre-release stance.
