# Agent Note: Resume the account's authoritative login task

Status: implemented

## Problem

Closing Harness Accounts or selecting another device unmounts the sign-in panel, but the device continues its OAuth task. A browser-only deduplication window cannot recover that task after reopening; starting again conflicts with the existing open row.

## Decision

For Claude, the OAuth relay and runner lifecycle below are superseded by [official CLI authentication](../architecture/2026-09-10-official-claude-authentication-terminal.md). The shared account rows and Codex behavior remain applicable.

The start endpoint returns the account's unexpired open `harness_account_login` with HTTP 200, or creates one with HTTP 201. Both decisions run under the existing account row lock and company authorization. Only creation launches a local driver. The database's unique open-account index remains the invariant; the browser has no parallel task cache.

Reopening the manager and clicking Sign in resumes the same link, callback input entry and status. Closing or changing device only stops that panel's polling. Explicit Cancel retains its existing cancellation contract. Unsubmitted authentication values remain component-local and are not persisted.

## Alternatives considered

**Extend the browser promise cache.** Rejected because another tab or a later mount still needs the database's current task, not a stale response. This supersedes the cache mechanism in [browser login deduplication](2026-09-03-harness-login-start-deduplication.md); that note retains the independent structured-error rendering decision.

**Cancel on unmount or replace the open task.** Rejected because the user may still be finishing the authorization page. [Explicit cancellation](2026-09-03-cancel-open-harness-login.md) remains the owner of that intent.

## Consequences

Concurrent starts and repeated navigation share one persisted task without extending its expiry. A gateway restart can still abandon its process-local OAuth runner; resuming a row does not recreate that runner. The user can cancel and start again.

Browser regressions use real account/login routes and PostgreSQL while substituting provider waiting-state publication. They prove task recovery and cancellation, not an external OAuth login. Driver completion and callback integration remain covered separately.
