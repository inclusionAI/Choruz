# Agent Note: Require owned resources for boundary-test evidence

Status: implemented

## Problem

Tests can pass without observing the production boundary they claim to protect. Global process kills and firewall changes also make manual fault scripts unsafe on a developer machine.

## Decision

Provisioning lease contention is tested through the production store using two real PostgreSQL connections, with a blocking-lock observation before the owner commits. Session integration tests require an explicit disposable database URL. Browser assertions target resources created by the test rather than global counts, and message-history coverage belongs to the browser journey instead of an opt-in script using a development account.

The unowned executor-kill, database-partition and network-delay scripts and their recovery verifiers are removed from `infra/host/chaos`. Its README names the focused test owners and the [owned recovery acceptance](2026-09-07-owned-recovery-acceptance.md). Lifecycle checks execute the harness-smoke guards; these guards are not evidence of a live harness session.

## Alternatives considered

**Keep unsafe scripts as optional tests.** Optional execution does not make global process and firewall mutations safe, and their presence overstates available recovery evidence.

**Replace the scripts with mock recovery checks.** Mocks cannot establish actual recovery after process loss or network interruption. Recovery requires a disposable, owned fault environment and final delivery assertions.

## Consequences

SQL negative controls and failed-delivery tests demonstrate that the assertions reject broken implementations. These focused checks cover their named boundaries; the separately owned recovery smoke establishes process-loss and database-proxy recovery. Neither establishes external provider availability. Unsafe retired entry points are not counted as passing tests.
