# Agent Note: WAL recovery reports persisted results, not attempted repairs

Status: implemented

## Problem

Startup recovery could report no incomplete turns after failing to read a WAL,
and counted a failed write as recovered. Operators could not distinguish a clean
scan from partial recovery using the pipeline metrics endpoint.

## Decision

The recovery owner records errors by operation, counts only persisted failure
markers, and publishes a final outcome through the existing metrics registry.
The existing SQLite regression now checks empty, corrupt, partially recovered
and repaired scans through the metrics handler without mocking storage.

## Alternatives considered

Failing startup would change availability and retry policy. This change retains
best-effort recovery and exposes degradation instead. WAL reconciliation does
not replace the session manager's retry lifecycle.

## Consequences

The [pipeline reference](../../../../docs/subsystems/message-pipeline.md#wal-recovery-evidence)
owns metric semantics. The [runbook](../../../../docs/operations/runbook.md#incident-wal-recovery-reports-errors)
owns operator response. Existing recovery acceptance requirements remain valid;
these counters alone do not prove a retried agent completed its work.
