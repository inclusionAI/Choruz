# Agent Note: Evolve bounded execution teams in the guidance search

Status: implemented

## Problem

Optimizing a single review prompt cannot compare different collaborator counts or information flow. A separate harness optimizer would duplicate evaluation, consent, activation and rollback, and could measure a team that production never runs.

## Decision

The existing candidate holds both executor guidance and a typed internal team. Team proposals change collaborator prompts, count and serial or parallel order, using the same training, validation and held-out gates as instruction proposals. The analysis Agent and evaluation definitions remain fixed. Structural search requires owner consent and reviewed recurrence evidence, or an already reviewed team. The [recurrence decision](2026-09-09-evidence-driven-execution-role.md) continues to own what qualifies as recurrence.

One device-local runner serves evaluation, structured DM and headless group execution. Members use the target account and model in fresh tool-free contexts. Serial members receive earlier findings; parallel members run independently and results retain declaration order. The existing Agent performs the final task. The total-agent limit includes that executor and prevents proposals from changing accounts, devices, permissions or public group membership.

## Alternatives considered

**Keep the single-reviewer runner beside a team runner.** This leaves two sources of execution and cancellation behavior. Saved reviewer configurations migrate into one-member teams, including rollback revisions and frozen populations.

**Let generated code redefine the loop.** Arbitrary control flow can bypass scope, cancellation and cost reservations. Typed team configuration reuses the execution boundary instead.

**Apply a proposed team after content review alone.** Measured mode requires real case execution and held-out acceptance before application. Normal search failure is not proof that the working harness is faulty or that another member helps.

## Consequences

Team members add latency and account usage. Calls are reserved by actual member count, with a total execution deadline before the final task. Failed or cancelled collaboration does not submit the foreground task. Prompts, team configuration and rollback share one revision pointer. The migration cancels unfinished old-format evaluations rather than replaying uncertain charged calls.

The evaluator measures self-contained text and JSON tasks, not arbitrary tool workflows or mathematical proof correctness. Successful small-suite selection does not establish production improvement. The [search decision](2026-09-09-reflective-learning-optimization.md) retains its independent selection and checkpoint rationale.
