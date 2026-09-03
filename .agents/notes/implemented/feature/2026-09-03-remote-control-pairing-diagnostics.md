# Agent Note: Remote Control Pairing Diagnostics

Status: implemented

## Problem

Remote Control pairing crosses the local API and the Cloud Gateway, but neither
side emitted enough correlated lifecycle information to identify where a failed
attempt stopped.

## Decision

The API generates one opaque `pairing_id` for each pairing and carries it in the
pairing capability. The API-owned host task logs issuance, host-socket readiness,
malformed or unavailable redemption, successful redemption, completion, expiry,
disconnect and failure. The Gateway logs credential validation, capability
lookup, pairing socket lifecycle and the pairing protocol message kind with the
same identifier.

The logs omit pairing credential secrets, tickets, ECDH material, session keys,
proofs, device names, close reasons, and payload contents. Credential
diagnostics are limited to the opaque identifier and its format.

## Alternatives considered

**Log the complete pairing credential.** Rejected because its secret component
is a single-use authenticator and log retention would enlarge its exposure.

**Log only in either browser dashboard.** Rejected because browser console
output cannot correlate a Cloud Gateway failure with the local API or Worker
process, and the local Dashboard need not remain open during pairing.

## Consequences

Operators can correlate an attempt with `pairing_id` across the local API and
Worker logs without handling pairing secrets. The Worker and local API process
must be deployed before their new diagnostic events appear in hosted pairing
attempts.

## Testing

Gateway ticket and capability validation cover the optional opaque pairing
identifier. The API integration test establishes a local Gateway socket and
asserts that the API-owned host remains connected after the credential request
has returned.
