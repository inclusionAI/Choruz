# Agent Note: One-code remote browser pairing

Status: implemented
Archived: 2026-09-03

## Problem

Remote Control required a person to enter an eight-digit code and then compare a second six-digit value on two screens. The second step made ordinary browser pairing stop for host-side intervention.

## Decision

The single-use eight-digit code authorizes the complete pairing. The host and browser retain the committed ephemeral ECDH exchange and encrypt the returned credentials with its derived secret, but the host releases those credentials immediately after the reveal instead of showing a six-digit comparison and waiting for approval.

## Alternatives considered

- **Keep the six-digit comparison**: rejected. It makes every pairing a two-step ceremony instead of the requested one-code flow.
- **Replace the comparison with a password-authenticated key exchange**: deferred. It could authenticate the ECDH transcript from a short code without a second screen, but requires a new audited protocol rather than a UI simplification.

## Consequences

- A person enters one code and pairing completes automatically.
- The code remains single-use and expires after five minutes, but this flow no longer detects a malicious gateway that substitutes an ephemeral key.
- The pairing and documentation tests cover automatic credential release and the user-visible one-code contract.
