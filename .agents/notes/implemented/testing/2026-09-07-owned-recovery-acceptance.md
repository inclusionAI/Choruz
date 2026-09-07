# Agent Note: Recovery acceptance owns the interrupted resources

Status: implemented

## Problem

Executor failure classification and readiness checks do not establish that an
interrupted user command reaches the conversation after recovery. Host-wide
process killing and firewall mutation also risk unrelated development work.

## Decision

The recovery smoke uses the existing isolated launcher and real API, pipeline
and PostgreSQL. Only the external CLI is a fixture. It terminates the exact
fixture PID after checking its executable path, then asserts the same command
retries and its reply is persisted. A private TCP proxy isolates a second
database-network fault without modifying host networking. The message API and
command state establish recovery, not a health check alone.

## Alternatives considered

**Restore global chaos scripts.** Process names and shared firewall rules cannot
establish resource ownership.

**Mock command status or advance leases manually.** That bypasses the scheduler
and writer whose recovery this test must establish.

## Consequences

CI runs the smoke in its DB/API job. Disposable ports and data permit concurrent
runs. Owned child exit and proxy close precede PostgreSQL cleanup. The fixture
does not certify provider authentication, model responses or host-wide outages.
