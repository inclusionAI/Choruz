# Agent Note: Online mail is not runtime control

Status: implemented

## Problem

Inviting a person into a conversation cannot grant the device-wide authority of
a Remote Control pairing. Reusing its executor would make an ordinary group
invitation a privileged login.

## Decision

Online transport uses the existing Cloud Gateway deployment and AES-256-GCM
envelope format, but has separate authenticated mailboxes and bilateral link
grants. A link allows encrypted delivery only; no Online frame dispatches an
HTTP request, terminal command or runtime operation.

## Consequences

The Worker authenticates the account from its bearer session, never from a
sender field. Link invitations contain a separate one-use admission grant;
encryption keys stay at the endpoints. The receiver checks the encrypted
message's sender, recipient, link and id against its transport envelope.

One account has one active Online mailbox socket. A second device replaces the
first explicitly; the client stops on replacement rather than reconnecting in a
loop. Distinct people sign in with distinct accounts. The Worker retains at
most 256 unacknowledged encrypted deliveries per account for seven days. The
application acknowledges only after its own durable processing and must keep
outgoing messages until an accepted event; socket writes are not delivery proof.

## Alternatives considered

- Reuse Remote Control's HTTP executor: it grants unrelated device capabilities.
- Treat socket writes as delivery: a disconnect between receipt and local
  persistence loses the message, so the receiver must acknowledge explicitly.
- Store plaintext at the relay: unnecessary for routing and incompatible with
  endpoint-only encryption.

## Evidence

`services/remote-control-gateway/src/online-transport.test.ts` runs the actual
Worker, D1 migrations and WebSockets. It covers invitation exclusion, ciphertext
delivery, acknowledgement/replay and revocation. Bypassing the production link
check makes the unauthorized-sender assertion fail. The Rust client's socket
test covers browser-independent reconnection and session expiry.

## Related

- [Online identity](2026-09-05-online-identity-is-not-device-control.md)
- [Host and remote subsystem](../../../../docs/subsystems/host-and-remote.md)
