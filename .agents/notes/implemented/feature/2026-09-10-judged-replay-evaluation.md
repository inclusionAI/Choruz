# Agent Note: Judged and isolated replay evaluation

Status: implemented

## Problem

Exact answer equality cannot assess open-ended objectives. A textual solution also cannot establish that proposed code executes correctly. Historical records do not necessarily retain the environment needed to repeat a task.

## Decision

The shared evaluator supports a reference plus acceptance criteria interpreted by a fixed, independent judge conversation. Candidate instructions never enter that conversation. Its version participates in the execution fingerprint. Inconclusive verdicts retain evidence without a zero score or candidate update. The optimizer consumes the runner's assessment instead of recomputing text equality.

Explicit replay environments bind a local image ID, bounded starting files and fixed verification commands. A disposable, non-root, network-disabled Docker container receives only those files. Model calls remain outside the container with native tools disabled; bounded command actions receive container observations. Verification is withheld from the solver and runs after its final answer. Original sessions and live workspaces are not resumed or copied implicitly.

This extends the [fixed evaluation decision](2026-09-09-fixed-learning-evaluation.md); its ownership, immutable suite, no automatic retry and no live-workspace side-effect guarantees remain. The [curation decision](2026-09-10-trace-dataset-curation.md) still owns grouping and source review. Extracted multi-step objectives remain valid when their inputs are self-contained; missing state is not fabricated.

## Alternatives considered

**Require deterministic answers for every task.** This excludes evidence-grounded writing and reasoning with valid alternative answers.

**Let candidates grade themselves or change the judge skill.** This changes the measurement rather than establishing an improvement.

**Run historical commands directly on the device.** Repeating external effects or altering the live project is not acceptable evaluation isolation.

**Reconstruct missing files from the final answer.** Such a reconstruction can leak the solution and does not establish a faithful starting state.

## Consequences

Judging adds a bounded model call and remains fallible. A replay uses an explicit step and wall-clock budget, requires a working local Docker engine and a preinstalled image, and cannot reach live services. It is not arbitrary browser or production-state replay. Reports preserve fixed-check failures and model assessments separately. Task-level claims are limited to the supplied environment and acceptance criteria.
