# Agent Note: One device owner for headless CLI execution

Status: implemented

## Problem

The pipeline and connector independently built and contained one-turn CLI
processes. Their executable resolution and resume-error handling diverged.
The executor package also exported an unused persistent Codex adapter and
created unused sandbox directories beside the real binding workspace.

## Decision

`choruz-host-runtime::headless` builds device-local invocations and owns their
process groups, cancellation and timeout cleanup. Both callers use its resume
error classifier and `choruz-agent-runtime::headless::parse_output` for native
answers and session identity. Platform task projection and durable session
updates remain in the pipeline; the connector retains HTTP leases and delivery.
Outbox transport differs deliberately and is supplied by each caller.

The binding workspace is the execution directory. The unused sandbox manager,
persistent CLI adapter and their dedicated tests are removed. The executor
package retains the SQLite WAL and startup recovery contract.

## Alternatives considered

**Keep two runners and duplicate regressions.** This retains the source of
device-specific fixes: a structured resume error or configured executable can
work locally and fail remotely despite both transports being healthy.

**Move persistence and leases into the device runner.** The controlled device
must not need controller database credentials. Fencing and durable writeback
remain at their existing database or authenticated HTTP boundary.

## Consequences

The runner resolves account directories only on the execution device. Native
processes retain account and host cancellation, isolated profiles, Claude fork
arguments and process-tree containment. Timeout and cancellation wait for child
exit before returning. No second workspace is created as bookkeeping.
The [runtime-host owner](../architecture/2026-09-04-device-operations-behind-runtime-host.md)
and [host-link transport](../architecture/2026-09-04-host-link-for-remote-devices.md)
remain authoritative for device selection and long-job queue semantics.
