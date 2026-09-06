# Agent Note: Remote recovery preserves startup, delivery and session ownership

Status: implemented

## Problem

A saved remote pairing could remain offline after a healthy HTTP startup: its
background TLS client panicked before any browser requested a new credential.
Transient capability failures could also terminate the bridge maintenance task.
An accepted remote attachment could remain unprocessed when the gateway and
pipeline used different runtime directories. File-only group turns were treated
as missing replies and replayed despite successful platform commands.

## Decision

The gateway installs its TLS provider before starting background services.
Missing transport capabilities for a live pairing are retryable failures, not
evidence that the pairing was removed. Connector startup retries transient
heartbeat failures with capped backoff; permanent client refusals remain errors.
This complements [connector supervision](2026-09-04-supervise-runtime-connectors.md)
and [API-owned pairing](2026-09-03-api-owned-remote-pairing.md).

The bridge probes both transport and rendezvous sockets with the gateway's
application-level ping every 25 seconds. Each socket owns its last-received
timestamp; more than 75 seconds without a frame ends the bridge session and
lets the existing maintenance loop reconnect. Traffic on one socket cannot
keep the other alive. A rendezvous close frame also ends the session immediately.

The local host launcher and pipeline watchdog resolve one service runtime
directory. The gateway's accepted command mirror and the pipeline's watched
directory must be identical. The [outbox mirror contract](../architecture/2026-09-04-remote-outbox-mirror.md)
continues to own receipt idempotency and file confinement. A successful group
turn may contain only platform commands, provided delivery succeeded or those
commands were durably spooled. A refusal or spool failure cannot become success.
Text-file shares use the named group's routing path rather than returning
their contents to the binding's default conversation.
The group-send writer stamps the binding's device identity for both text and
attachments. Agent-supplied metadata cannot replace that identity.

Managed Claude processes remove inherited parent-session markers in both
headless and PTY launches, without removing account credentials. A terminal
that launched Choruz must not make every managed agent its own child session.

Terminal attachments capture replay and subscribe under the PTY reader's lock.
The first attachment consumes raw startup bytes; subsequent attachments use a
bounded VT screen snapshot. A controller restart must not require new CLI
output before its still-running remote terminal becomes visible.

Reply sync events preserve reply metadata through the database trigger, so
live device labels agree with loaded history. The dashboard restores a valid
Company selection independently of its last conversation; an explicit
conversation link takes precedence.

## Alternatives considered

**Ask users to re-pair or restart both devices.** Rejected because a stored
pairing is valid across controller restarts. Manual pairing hides a dead
background task rather than repairing its lifetime.

**Treat transport traffic as proof that the bridge is healthy.** Rejected
because a half-open rendezvous socket cannot advertise the transport room to a
returning browser, even while the data socket still answers. Both connections
need independent liveness evidence.

**Require a text reply after every file or board command.** Rejected because
silent commands are valid protocol actions and replaying them repeats side
effects. Delivery acknowledgement, not extra prose, determines acceptance.

**Copy queued files between runtime directories periodically.** Rejected because
it creates a second delivery mechanism and receipt ownership problem. Producers
and consumers must share the same configured root.

## Consequences

HTTP readiness alone is insufficient remote acceptance evidence. Regression
coverage includes cold-process TLS startup, transient heartbeat recovery, the
reply sync feed, host launch directory agreement, file-only turn completion,
parent-session isolation and Company reloads. Live acceptance must also observe
the same stored device recovering and an attachment reaching its conversation.

The socket regression runs the production bridge against two owned WebSockets,
advances the Tokio clock, and keeps one side responsive while the other stays
silent. Encrypted executor replies and targeted room offers acknowledge reads.
The hosted remote-editor test reloads with the saved pairing before saving B's
file. Neither test proves the cause of an uninstrumented historical cloud outage.

Terminal replay restores the visible screen, not unlimited scrollback. Both
local and linked terminals use the shared runtime implementation; remote screen
recovery requires deploying that runtime to the connector as well as the API.

The runtime mirror and its receipts remain durable only for the lifetime of
their configured directory. This change does not add host boot persistence,
change credential expiry, or make platform side effects transactional with a
harness turn.
