# Agent Note: Online identity does not authorize device control

Status: implemented

## Problem

People using different Choruz installations need a shared login identity. A
local principal is only meaningful to its own installation, while Remote
Control credentials authorize control of a device rather than participation
in a conversation. Reusing either as a public account would confuse these
security boundaries.

## Decision

The hosted Worker uses Better Auth and a separate D1 identity store. The local
API binds its authenticated human to one cloud session in `online_identity`,
scoped by principal and workspace. Cloud identity is global and has no Company
workspace; the local binding and local audit records retain workspace scope.
The browser receives account metadata, not the cloud session credential.

Registration and sign-in never create a runtime host, Remote Control pairing,
Company membership or group membership. Local usage still requires no Online
account. Authentication success is called `signed_in`, not presence. Logout
revokes the cloud session before deleting the observed local binding; concurrent
sign-ins cannot silently overwrite it.

## Alternatives considered

**Use local account passwords globally.** Local account storage does not provide
the hosted identity and public password lifecycle required here.

**Reuse a Remote Control bearer.** It grants broad device operations and is not
a group invitation or a human identity.

**Write another password/session implementation.** Better Auth already owns
hashing, expiry and revocation, including native D1 support.

## Consequences

The account service sees login credentials over HTTPS and stores password
hashes; this must not be described as an end-to-end-encrypted account exchange.
Chat encryption is a separate transport concern. Local server storage contains
the session credential and must be protected like other server credentials.
An offline logout reports failure rather than claiming remote revocation.
Email is an unverified login identifier, not proof of mailbox ownership.

The existing [runtime-device decision](2026-09-03-unified-runtime-devices.md) and
[dashboard transport](2026-09-03-remote-dashboard-transport.md) remain active:
neither is replaced by human Online authentication.

## Testing

`online-auth.test.ts` runs the production auth implementation and SQL schema
against SQLite for wrong passwords, token isolation and revocation. Gateway
PostgreSQL tests pin local actor scoping, conflict handling and credential-free
audit rows. `online.spec.ts` traverses the browser, local API and actual Worker/D1
for registration, reload, sign-out and sign-in. These checks do not establish
cross-device group collaboration.
