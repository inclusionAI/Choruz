# Agent Note: Run evaluations of different bindings side by side

Status: implemented

## Problem

The evaluation worker claimed one evaluation case per tick and awaited it before claiming another. Every case dispatches the task to the binding's device and may wait for a judge, so evaluations from all owners formed one global queue: a slow device, a long replay or a stalled judge delayed every other binding's measurement and automatic application. With several owners learning at once, a reviewed candidate could wait well over a minute behind unrelated work. The browser acceptance test for remote learning failed intermittently under parallel load because its candidate was still queued when it checked for activation.

## Decision

The worker keeps up to `CONCURRENT_EVALUATIONS` (four) cases in flight in a `JoinSet` and claims more as slots free, matching the [learning worker](2026-10-19-resumable-learning-stages.md)'s bounded admission. `claim_experience_evaluation` already moves an evaluation to `running` under a lease with `FOR UPDATE SKIP LOCKED`, so one evaluation still runs one case at a time, in order, across workers and processes. A case's result commit failure is logged per case instead of stopping the loop.

## Alternatives considered

**A separate worker per binding or device.** Unbounded executors could exhaust device slots and model quotas; a fixed bound matches the existing learning admission.

**Parallel cases within one evaluation.** Cases share an ordered optimization search and its budget; running them out of order would change which candidate a later case measures.

## Consequences

Up to four evaluation cases may hold devices and judge calls at once, raising peak usage for owners who start several evaluations together. A slow evaluation still delays its own later cases. The regression test holds one binding's evaluation inside its device call and requires another binding's evaluation to complete meanwhile; with the previous serial worker it times out.
