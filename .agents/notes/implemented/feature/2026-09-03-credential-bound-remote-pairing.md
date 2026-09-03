# Agent Note: Bind Remote Control pairing to one opaque credential

Status: implemented

## Problem

An eight-digit Remote Control code is an online guessing target when the Remote page has no account login. The committed ECDH exchange protects a passive observer but does not prove that either peer received the code through the intended channel, so the CLI adds a second comparison code and approval step. Putting the first code in a URL query also exposes it to HTTP logs and referrers.

## Decision

Remote Control issues one single-use credential with the exact shape `v1.<128-bit base64url id>.<128-bit base64url secret>` and a five-minute lifetime. The user copies or pastes that value once. The identifier is the only credential component sent through the Cloud Gateway lookup; the secret stays in the browser URL fragment or input and is never sent in an HTTP request.

Both peers mix the credential secret into the P-256 ECDH HKDF salt. The host and browser then exchange role-bound HMAC proofs over both ephemeral public keys before the authenticated local host redeems the credential and sends encrypted device credentials. Possession of the one credential therefore replaces the separate six-digit comparison without weakening the gateway-substitution check. Pending eight-digit Remote Control pairings are intentionally invalidated; already paired devices keep their stored session credentials.

The local database stores only `credential_hash`, keyed by `CHORUZ_REMOTE_CONTROL_PAIRING_SECRET` or the session secret. The Worker stores the opaque identifier, room, expiration and diagnostic identifier, applies its per-address limiter, and consumes the identifier when the first client joins a live host so an early connection race can retry but each pairing permits only one protocol attempt. Logs contain the identifier and format result but never the credential secret, derived key, proof, ticket, or encrypted payload.

## Alternatives considered

**Keep the eight-digit code and add stronger rate limits.** Rate limits reduce online guesses but retain a small authentication space and make distributed guessing an operational policy problem.

**Offer both the opaque credential and a shorter manual fallback.** A fallback preserves the weaker authentication path and doubles the protocol and support surface, so there is only one accepted credential format.

**Keep the six-digit comparison after the opaque credential.** The role-bound proofs already verify possession of the secret that arrived outside the gateway; another human confirmation adds ceremony without adding a distinct trust signal.

**Put the complete credential in a query parameter.** It makes links convenient but sends the secret to dashboard and gateway HTTP infrastructure. A fragment gives the browser the same launch behaviour without server disclosure.

## Consequences

Remote pairing has one user action and at least 256 random credential bits split between lookup and proof material. The longer value is designed for copy and paste rather than transcription. A malformed, expired, consumed, or unproved credential fails closed, and a credential issued by an older host cannot pair with the new Worker. The UI, CLI, API, Worker and database migration must deploy as one protocol version; staging acceptance covers the hosted Worker before production deployment.

## Testing

Rust tests pin credential shape, hashed one-time redemption, credential-bound ECDH and role proofs. Web tests pin fragment-only launch, identifier-only gateway URLs, mutual proof, encrypted completion and malformed input. Worker checks pin capability validation and redirect behaviour; the Remote and Machines browser specs pin the paste flow.
