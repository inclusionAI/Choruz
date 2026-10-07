# Agent Note: Separate device tool setup from platform execution

Status: implemented

## Problem

Browser and desktop tool setup depends on the entire host runtime even though its operations use only device paths and subprocesses. Executor error conversion also imports the PostgreSQL-backed session manager. The pipeline constructs an unused passthrough tool gateway, suggesting protection for native CLI effects that it never intercepts.

## Decision

This package boundary supersedes the host-runtime ownership of setup in
[device computer-use skills](../feature/2026-09-07-device-computer-use-skills.md).
That note remains active for account isolation, opt-in installation and consent.

`choruz-computer-use` owns device tool installation and diagnostics. Gateway and host dispatch call that implementation. Process-group containment belongs to `choruz-agent-runtime` and is shared by terminals, structured sessions, analysis and installation. The executor has no session-store dependency. The pipeline does not construct the unused tool gateway; native CLI tools remain owned by their Harness.

The existing `choruz-tools` library is not a claim that native tools are intercepted. Its SQL journal remains unchanged and has no active pipeline consumer.

## Alternatives considered

**Duplicate a standalone installer.** Rejected because platform repairs and standalone setup would drift.

**Abstract the unused journal integration.** Rejected because no production execution invokes it; adding persistence ports would not establish interception or replay safety.

## Consequences

Device tool setup can be consumed without learning, community, evaluation or the HTTP server. Native history support in agent-runtime still carries its local SQLite dependency. This boundary is not a plugin installer or a standalone background service; unattended lifecycle composition remains separate work.
