# Agent Note: Wake background workers when their work becomes due

Status: implemented

## Problem

The learning, evaluation and behavior workers only looked for work on fixed polls: learning claimed due checks every 15 s, automatic evaluations were queued every 15 s, and behavior preparation claimed one event per 15 s. Saving learning settings, saving a reviewed candidate or recording behavior evidence therefore waited up to one full poll per stage, and a burst of behavior events took 15 s each. A remote learning acceptance run spent most of its minute-long wait between polls rather than on work.

## Decision

`wakeup::WakeupHub` is the single LISTEN fan-out for the gateway, parameterised by channel: `SYNC_CHANNEL` (`choruz_sync_change`) for the dashboard sync feed and `WORKER_CHANNEL` (`choruz_background_work`) for background workers. The [background-work triggers](../../../../migrations/V070__background_work_wakeups.sql) notify `learning` when an enabled, unleased policy becomes due, `evaluation` when a candidate revision is saved or an evaluation case is queued, and `behavior` when behavior evidence is recorded. Payloads name a worker and carry no state.

`wakeup::next_round` returns when the worker's poll interval elapses or a notification names it, and reports which. The learning worker claims on either; the evaluation worker queues automatic optimizations immediately when notified and otherwise every 15 s; behavior preparation drains every claimable event per round. Polls remain the fallback: a missed notification, a lagged receiver or a closed listener costs at most one interval, never work.

## Alternatives considered

**An in-process notify from each writer.** Leases already coordinate several gateway processes through PostgreSQL; an in-process signal would miss work written by another process or by SQL that bypasses the Rust writer.

**A second LISTEN implementation for workers.** The sync feed's listener already handles reconnect backoff and the startup race; duplicating it would split that behaviour across two owners.

**Shorter polls.** Every idle worker would query the database several times a second and the latency floor would remain.

## Consequences

The gateway holds two LISTEN connections. Learning, evaluation queueing and behavior preparation start within the notification round trip after their trigger commits. A retry scheduled for the future, such as a deferred busy-source check, still waits for the next poll because no row change marks it due. Tests cover `next_round` choosing between notification and interval, each trigger notifying its worker, and a newly enabled policy being checked within seconds; removing the migration or the learning worker's wakeup fails them.
