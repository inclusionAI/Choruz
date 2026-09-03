# Agent Note: Dashboard-to-host remote pairing

Status: implemented
Archived: 2026-09-03

## Problem

A Choruz host prints a short-lived pairing code, but an operator already using another Choruz Dashboard needs a direct place to enter it. Sending the operator to find a separate URL makes the direction of pairing unclear. Host startup also has to work when a release build unifies several rustls providers and when embedded PostgreSQL initializes on slow storage.

## Decision

The Remote Control dialog accepts the eight-digit code printed by another Choruz host and hands the browser to the configured hosted dashboard with that code and device name. The hosted dashboard starts its existing committed ECDH handshake from those launch parameters. The derived six-digit value remains a comparison check on both endpoints; it is not entered as another pairing code.

The `choruz` CLI explicitly installs the rustls `ring` provider before creating HTTP or WebSocket clients. Embedded PostgreSQL commands use a 60-second timeout, with a positive `CHORUZ_POSTGRES_COMMAND_TIMEOUT_SECS` override for unusually slow or fast environments.

## Alternatives considered

**Keep the pairing input only on the hosted dashboard.** This preserves fewer web controls but leaves an operator on device A without a discoverable path from the Choruz interface they are already using.

**Remove the six-digit comparison.** A single visible code would look shorter, but it would discard the explicit key-substitution check provided by the committed ECDH transcript.

**Rely on rustls provider inference.** Dependency feature unification can enable both `ring` and `aws-lc-rs`, so inference is not deterministic for release builds.

**Leave PostgreSQL's five-second crate default.** It works on fast local disks but rejects valid first starts on slower disks and network-attached storage.

## Consequences

Pairing starts from either a standalone browser or an existing Choruz Dashboard while using one encryption protocol. The transition to the remote workspace still occurs on the hosted dashboard origin, where its encrypted credentials remain isolated. PostgreSQL startup can wait longer on genuine failures, while operators that need another bound can configure it explicitly.
