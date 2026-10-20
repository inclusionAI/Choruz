# Agent Note: Device revocation fences execution and terminal lag recovers a snapshot

Status: implemented

## Problem

Removing a device must not reinterpret its paths and unfinished commands as
controller-local work. Losing terminal bytes must not leave a misleading screen
or restart native work merely to repair its display.

## Decision

Runtime-host revocation commits its token fence, disabled bindings and terminal
command outcomes before best-effort device cleanup. Bindings retain device
identity and conversations retain history. Disabled bindings cannot enqueue
commands. Explicit assignment is separate; it does not retry cancelled commands.
Assignment validates the selected account against the destination device. An
account-bound Agent must be recreated through the existing Create Agent flow
with the destination's workspace and account; it never inherits a remote profile
or silently switches to the destination's default login.
This supersedes the online-only revocation constraint in the
[structured DM decision](../feature/2026-09-06-structured-agent-dm.md), not its
live-device assignment guards or session-identity isolation.

The connector treats command-heartbeat 401, 403, 404 and 409 responses as loss
of execution authority, kills the process container and waits for the child
before reporting cancellation. Transient network and server errors remain
retryable. Controller fencing does not claim that an unreachable device has
already stopped; revocation prevents new authorization and late commits.

The [host link](../architecture/2026-09-04-host-link-for-remote-devices.md)
terminates an attachment on output lag. Device-side lag emits terminal_resync;
controller-side queue overflow marks the same recovery state. The browser socket
closes with 1012, the bridge preserves the running terminal, and reconnection
resets the emulator before applying the device's current snapshot. Ordinary
browser disconnect retains its existing process-cleanup contract.

## Alternatives considered

**Requeue removed-device work on the controller.** Device-local paths, profiles
and tools are not interchangeable; an explicit move must precede new work.

**Require the device online before revocation.** Lost devices still need an
immediate durable authorization fence. Cleanup acknowledgement is distinct from
revocation and cannot be promised while the device is unreachable.

**Skip terminal frames and keep rendering.** ANSI bytes carry state as well as
text, so a truncation marker cannot repair a partially applied control sequence.
Snapshot recovery preserves native execution while restoring the display.

## Consequences

Removed-device Agents remain readable but execution is disabled. Active work may
continue until a reachable connector observes rejection; the controller never
replays it automatically. Terminal recovery restores screen state, not an
unbounded historical log. Existing host-link and structured-session notes remain
active because their device ownership and exact-session guarantees still apply.

## Testing

The runtime-host pairing regression checks offline removal, retained device
identity, disabled bindings, rejected admissions and no runnable revoked-host
commands. The connector regression runs a real native fixture process through
transient and rejected HTTP heartbeats and observes child reaping before its
completion report. The host-link recovery regression saturates a controller
consumer, reattaches through the device dispatcher and checks the same live
process and its restored screen.
