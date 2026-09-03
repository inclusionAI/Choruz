# Agent Note: Keep Remote Control pairing alive in the host API

Status: implemented

## Problem

A Remote Control credential was returned while its host-side Gateway socket
belonged to a Dashboard modal or to the short-lived `choruz` CLI process. A
component unmount, page navigation, shell exit or process teardown could close
that socket while the five-minute credential still looked valid. The remote
browser then reached the Gateway with a valid credential, but the pairing room
had no host and rejected the connection.

`choruz start` also waited on the foreground `choruz-server` child, so a remote
operator could not obtain a credential and let the command return while leaving
the host available for control.

## Decision

`POST /v1/remote-control/pairings` creates the one-time host capability and
connects its Gateway WebSocket before returning the credential. The API-owned
task in `services/choruz-api-gateway/src/remote_control_pairing_host.rs` performs
the committed P-256 ECDH handshake, redeems the credential internally, and
stays alive until pairing completes or the credential expires. Dashboard and
CLI callers only request and display the credential; their lifetime does not
own the host socket.

`choruz start` starts `choruz-server` in a detached OS session, records its PID
and appends its logs under the platform data directory's `choruz/` directory.
It waits for the server handshake, requests a ready pairing credential, prints
it, and exits. A later invocation reuses the ready loopback host and creates a
new credential.

## Alternatives considered

**Keep the host socket in the Dashboard and add reconnect logic.** Rejected
because browser navigation and a closed local Dashboard are normal states for a
headless server. Reconnect would still make a user-interface component the
owner of server availability.

**Keep the pairing handshake in the CLI and daemonize only after pairing.**
Rejected because the command would still have to remain open until the remote
browser arrives, contrary to the one-command background-host workflow.

**Let the remote browser enter an empty Gateway room and wait for a host.**
Rejected as the primary fix because it masks the missing host owner and leaves
credential readiness ambiguous. The API now proves the host is connected before
advertising the credential.

## Consequences

Closing the local pairing modal or the shell that ran `choruz start` no longer
invalidates a displayed credential. Credential creation now includes a live
Gateway dependency and fails before displaying a code if the host cannot join
the room. A detached host continues consuming local resources until it is
stopped by the operating system or a host-management command; its PID and log
file make that lifecycle observable.

The pairing handshake is now a Rust server responsibility as well as a browser
wire contract. Gateway integration coverage asserts that the host socket opens
before the credential response and remains connected after that response.
