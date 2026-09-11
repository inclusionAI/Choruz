# Agent Note: Trace dataset curation

Status: implemented

## Problem

A growing personal regression corpus needs interpretable categories, representative sampling, duplicate control and periodic evidence review. Otherwise frequent paraphrases dominate scores and obsolete answers retain authority.

## Decision

The existing fixed analyst emits reviewed classifications and equivalent-objective links. Task type, capability, step structure and outcome remain separate. Deterministic connected groups use those links, normalized inputs and shared evidence; conflicting groups do not score. Storage assigns a persistent group anchor to every member, so pruning an older member cannot promote exposed work into a holdout. Incoming analyst anchors are ignored. Round-robin categories choose representatives under the existing budget. No grouping decision deletes a source record.

Daily retained-source sweeps reuse the existing leased background worker and durable cursors. They resume interrupted paging and withdraw answers not revalidated at sweep completion. Analysis reports transactionally freeze the working corpus and quality summary; evaluation suite names carry its content version, checked under the same policy lock. Candidate optimization cannot alter this analysis policy.

## Alternatives considered

**A separate dataset service and scheduler.** These would duplicate scoped storage, lease ownership, model dispatch and report history already owned by learning.

**One label for both outcome and fault.** Missing user information and normal research failure do not imply Agent error. The outcome records progression while the rationale preserves the distinction.

**Drop failed or difficult work.** This would reward an easier benchmark rather than better execution. Unsupported answers remain documented but cannot score.

## Consequences

Semantic grouping remains a model-reviewed judgment, not a proof of equivalence. Daily rereads consume analyst calls; source retention bounds what can be revalidated. Versioned summaries describe the bounded working corpus, not all archived traces. Historical scores are not unseen-task performance. The [trace admission](2026-09-10-trace-evaluation-cases.md) note retains its source-grounding, consent and tool-free execution decisions.
