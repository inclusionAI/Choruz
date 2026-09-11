# Agent Note: Trace-derived evaluation cases

Status: implemented

## Problem

Objective analysis and measured optimization need a shared data path; manual answer entry prevents background learning from evaluating ordinary historical work.

## Decision

The fixed analysis procedure emits objective cases alongside its existing summary. [Task quality review](2026-09-10-task-quality-review.md) owns per-task admission, blind trial diagnostics and repaired-task rechecking; it replaces the original batch-level admission verdict. Cases and withdrawals share the analysis report transaction and workspace boundary. The optimizer reads a bounded latest-per-objective corpus and freezes the suite and source evidence before execution. Corrections cancel pending runs using changed evidence; completed runs remain historical records.

Objective identifiers keep retries together; [dataset curation](2026-09-10-trace-dataset-curation.md) owns grouping, sampling and periodic review. Duplicate inputs and overlapping evidence do not become independent tests. An insufficient corpus waits rather than inventing answers. The existing exact-text and JSON checks retain their tool-free scope. An unchanged instruction can seed measured search; this does not classify normal exploration as an error or bypass team-evolution consent.

## Alternatives considered

**A second segmentation service.** It would duplicate the existing objective analysis and introduce inconsistent episode boundaries.

**Use every final Agent response as an expected answer.** Completion claims are not correctness evidence; later user corrections must be able to withdraw them.

**Replay arbitrary historical environments.** The current executor cannot reconstruct files, external state or proof checking. Such objectives remain unevaluable instead of receiving misleading scores.

## Consequences

Dataset review consumes additional analyst calls. Historical scores cannot establish unseen-task generalization. Grouping depends on the fixed analyst identifying objective boundaries correctly; deterministic checks prevent exact overlap, not every semantic paraphrase. The bounded corpus is a working regression set, not an exhaustive archive. Original reports remain the provenance owner.

The [measured application](2026-09-09-measured-learning-application.md) and [fixed evaluation](2026-09-09-fixed-learning-evaluation.md) decisions retain their consent, selection and execution invariants.
