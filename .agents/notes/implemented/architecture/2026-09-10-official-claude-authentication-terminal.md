# Agent Note: Official CLI owns Claude authentication

Status: implemented

## Problem

The Claude account flow interpreted OAuth messages and exposed an application-owned callback form. Web model discovery also pulled in an SDK despite an existing device-side CLI probe. These duplicated ownership and complicated distributing Choruz independently of provider software.

## Decision

Choruz source uses Apache-2.0. NOTICE retains the incorporated MIT permission notice; third-party software and trademarks keep their own terms. Packaging carries both files. This does not grant redistribution rights to proprietary CLIs or certify compliance with their service terms.

Claude sign-in runs the installed, unmodified CLI with no Agent arguments in a private terminal. Choruz selects the account profile, transports terminal bytes and checks account identity on Done. It never interprets the terminal's login URL or code, and does not persist the terminal in activity or trace storage. The device link has a distinct authentication launch request so an old host cannot silently apply Agent launch behavior. Disconnect, cancellation and expiry stop the terminal. A second viewer cannot reuse it.

Model discovery uses the device CLI integration, not the Agent SDK. Missing models or quota cannot invalidate a successful identity check. The existing exact-usage refresh remains separate. Codex's browser flow is unchanged.

This partially supersedes [shared login handoff](../feature/2026-09-03-local-harness-login-handoff.md) and [login recovery](../bug-fix/2026-09-05-resume-open-harness-login.md): their account row, isolation and Codex recovery decisions remain; Claude no longer uses their OAuth relay or persistent runner.

## Alternatives considered

**Only change the license.** Rejected because it leaves SDK distribution and application-owned Claude authentication as separate unresolved integration concerns.

**Parse official terminal output into another callback form.** Rejected because it recreates the authentication intermediary this change removes.

**Use the ordinary Agent terminal unchanged.** Rejected because its arguments, outbox and trace capture are inappropriate for authentication.

## Consequences

Claude users interact with its terminal and click Done afterward; a closed terminal must be reopened if sign-in was not finished. Default and isolated profiles remain supported. Provider binaries remain user-installed and governed by provider terms.
