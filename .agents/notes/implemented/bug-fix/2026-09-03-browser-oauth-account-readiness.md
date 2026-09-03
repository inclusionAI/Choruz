# Agent Note: Browser OAuth and fail-closed Harness account lifecycle

Status: implemented

## Problem

Codex accounts used device-code login while the Harness's normal experience used browser OAuth, so local and remote accounts exposed different steps. A successful Claude OAuth callback could be marked failed when the first account snapshot arrived before its model catalog. Codex rate-limit buckets lost their server-provided names, which rendered unrelated weekly limits as duplicates. Removing an account was blocked by its bindings even though the user had explicitly chosen to stop managing those credentials.

## Decision

Codex uses the app-server `chatgpt` browser login on every device. Local login completes through the browser's loopback redirect. For a remote host, the user opens that authorization link on the controlling computer and pastes the complete localhost callback URL; the connector verifies that its state matches the authorization request, reconstructs the callback from code and state only, and sends it only to the loopback listener from the original request. It never requests an arbitrary URL.

The authentication-versus-catalog decision in this note is superseded by [Authenticated Harness accounts survive catalog failures](2026-09-03-harness-authentication-snapshot-separation.md). Codex quota parsing keeps a non-default bucket's `limitName`, labels each duration within that bucket, and removes only windows with identical duration, reset time and used percentage. Removing a Harness account locks it, disables all runtime bindings that reference it, and then disables the account in the same database transaction.

## Alternatives considered

**Keep device-code login for remote Codex accounts.** Rejected because device-code login is a fallback, not the requested normal browser flow, and it makes account setup depend on where the Harness runs.

**Forward the pasted callback URL as-is.** Rejected because a crafted URL could turn the connector into an arbitrary HTTP client. Only the code and matching state are copied onto the loopback URL originally issued by Codex.

**Make the OAuth login wait indefinitely for a Claude model catalog.** Rejected because authentication and catalog discovery are separate upstream operations. Their current boundary is owned by [Authenticated Harness accounts survive catalog failures](2026-09-03-harness-authentication-snapshot-separation.md).

**Flatten or guess Codex quota names.** Rejected because multiple products can expose overlapping durations. The server-provided bucket name is the only exact distinction.

**Leave bindings active after account removal.** Rejected because those Agents would reference credentials the UI no longer manages. Deleting the Agent principals is also unnecessary; disabling their bindings preserves identity and history for later reconfiguration.

## Consequences

Remote Codex login includes one copy-and-paste step when the controlling browser cannot reach the remote loopback listener. Existing in-flight device-code rows can still render until they expire, but every new login uses browser OAuth. Codex quota cards are longer when a named product has both five-hour and weekly limits. Agents whose account is removed remain visible but cannot execute until assigned an active account.
