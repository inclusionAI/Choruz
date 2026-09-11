# Agent Note: Measured learning application

Status: implemented

## Problem

An optimizer's selected winner is evidence, not permission to change an Agent.
Automatic learning also needs a durable decision when evaluation or review fails,
without repeatedly spending on the same candidate or changing its own criteria.

## Decision

The owner optionally selects a manual or [trace-derived suite](2026-09-10-trace-evaluation-cases.md) and per-run search budget in the
existing learning settings. The analysis worker then stores reviewed candidates
without activating them. The existing evaluation worker admits each candidate
once per policy generation. Automatic application requires separate consent,
strict validation improvement, held-out non-regression and fixed content review.
Held-out results cannot choose a different winner or generate another proposal.

Final review reserves a call under the evaluation lease. Application creates a
revision and updates the existing selection pointer atomically, fencing older
analysis. It preserves source provenance and records the first applied problem
intervention without creating another occurrence. Clear and restore use the same
selection owner. Failed or uncertain runs retain evidence without automatic replay.

## Alternatives considered

**Activate the best validation score immediately.** This ignores held-out
regressions and whether a generated instruction retains justified guidance.

**Generate the evaluation criteria from each candidate.** This lets the search
change its own standard. The suite is independent of candidate proposals and frozen per run.

**Replace the live harness during evaluation.** Running turns retain their
captured guidance. Only later submissions receive an applied revision.

## Consequences

The suite measures tool-free response behavior, not arbitrary coding or research
success. Tiny suites provide limited evidence. Budgets bound calls per revision,
not tokens, money or lifetime usage. Disabling learning fences pending work but
cannot undo a provider call already dispatched. Content-review-only learning
remains an explicit mode when no fixed suite is supplied. This decision composes
the [search](2026-09-09-reflective-learning-optimization.md) and
[evaluation](2026-09-09-fixed-learning-evaluation.md) boundaries; neither is replaced.
