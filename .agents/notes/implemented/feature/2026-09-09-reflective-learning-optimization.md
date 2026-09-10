# Agent Note: Reflective learning optimization

Status: implemented

## Problem

A single proposed instruction can improve one case while losing useful behavior elsewhere. Content review and a final average alone do not provide an iterative search or preserve complementary candidates.

## Decision

The [fixed evaluation boundary](2026-09-09-fixed-learning-evaluation.md) remains authoritative. A checkpointed search samples its validation Pareto frontier, reflects on training rollouts, and admits strict minibatch improvements to full validation. Optional merge proposals combine two frontier parents. Only the named guidance component can change. The final held-out comparison occurs after winner selection.

The existing evaluation queue owns execution, fencing and history. It reserves calls before dispatch and persists completed actions, allowing queued work to continue without replaying completed actions. An uncertain in-flight action fails the run rather than silently refunding or repeating spend. Selection returns evidence, not permission to activate a revision.

## Alternatives considered

**Keep only the highest average candidate.** This discards specialists whose complementary behavior can support a useful merge.

**Give the proposer the complete suite.** Validation and test answers would become construction data rather than independent evidence. Only training feedback enters reflection.

**Cache every result implicitly.** Stochastic tasks can then reuse an unusually favorable or unfavorable parent measurement. Fresh comparisons are the default; explicit caching trades repeated measurement for cost.

**Build another scheduler.** The evaluation queue already owns scope, leases and policy invalidation; a second scheduler would duplicate those guarantees.

## Consequences

Search budgets bound metric and proposal calls; model-call reservations include visible preflight execution. Checkpoints and candidate ancestry make selection inspectable. The adapter remains restricted to tool-free text and JSON tasks. It is not a general tool-workflow optimizer, and a validated winner is not evidence of improved production research until separately applied and observed. The existing trace-analysis decision and fixed-evaluation note retain their independent rationale; this note supersedes neither.
