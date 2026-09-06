# Agent Note: One calendar evaluator for scheduled jobs

Status: implemented

## Problem

Scheduled jobs can run at the wrong hour when creation and dispatch approximate cron as an elapsed delay. Independent evaluators also let a timezone-only edit retain the wrong next occurrence.

## Decision

`choruz_application::schedule::next_run_at` owns validation and next-occurrence calculation for the gateway, outbox handler and scheduler. Croner evaluates calendar expressions with chrono-tz IANA zones; callers supply the reference instant. Recurring dispatch chooses the next future occurrence instead of replaying missed runs. The gateway recomputes on timezone-only edits as well as expression edits.

## Alternatives considered

**Keep per-entry calculations.** This duplicates calendar semantics and permits creation and dispatch to disagree.

**Implement cron parsing locally.** A hand-written calendar engine adds maintenance for field matching and timezone transitions that existing libraries already own.

## Consequences

The three approximate evaluators are removed. Invalid expressions, zones, non-positive intervals and past one-shot timestamps return validation errors instead of creating unschedulable jobs. Existing polling cadence and command delivery remain unchanged. Fixed-clock tests cover calendar and daylight-saving offsets; database route and dispatch tests assert stored next-run instants, including timezone-only edits and agent outbox creation.
