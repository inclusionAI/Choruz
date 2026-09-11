# Agent Note: Measured task curation

Status: implemented

## Problem

Task outcome labels do not measure how a current Agent performs on a task. Pooling candidate-search scores or changed executor settings produces misleading difficulty, while unconstrained augmentation changes the problem and can contaminate holdouts.

## Decision

Use completed baseline results from the existing evaluation store, scoped to current executor and active guidance and exact task snapshot. Deduplicate answers within each run and retain sample counts. Training sampling balances category and observed difficulty; held-out scores never enter this selection. Low sample counts remain explicitly insufficient.

The existing analyst may propose one paraphrase, with no new task identity or answer. The fixed task reviewer independently checks it against its original after a blind trial. Only training groups use admitted variants, as a replacement input with one vote. Invalid variants leave originals intact. This extends [task review](2026-09-10-task-quality-review.md), not its admission standard or the [curation partition](2026-09-10-trace-dataset-curation.md).

## Alternatives considered

**Ask the analyst to guess difficulty.** This confuses apparent complexity with measured executor performance.

**Count every optimizer answer.** Adapted candidates and repeated observations would distort baseline rates.

**Generate many new problems or mutate holdouts.** This changes source meaning, overweights some objectives and allows benchmark improvement without Agent improvement.

## Consequences

Counts are bounded historical observations, not statistical guarantees or a universal difficulty ranking. A configuration or task correction starts a new measurement scope. Variants require up to two additional asynchronous calls; no generator service or scheduler is added. Original source retention, fixed judges, sensitive exclusion and rollback remain unchanged.
