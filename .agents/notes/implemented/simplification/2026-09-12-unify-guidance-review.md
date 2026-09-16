# Agent Note: Unify immutable guidance review

Status: implemented

## Problem

Seed review asks for a full analysis and an exact instruction echo, while optimized application uses a dedicated decision response. Both judge an immutable candidate, but maintaining two response protocols makes unrelated analysis fields a condition of seed acceptance. Test-only scoring and projection helpers also expand production APIs without production consumers.

## Decision

Both independent review calls use the host runtime's `ReviewExperience` operation and fixed guidance-review procedure. Its bounded response contains acceptance, a reason, source references and addressed problem keys, not a replacement instruction or analysis. Seed acceptance still requires verified references and exact proposed-problem coverage. Optimized application requires references from its reviewed seed, no new addressed-problem claims, measured score gates and owner consent. Neither review can rewrite the candidate. Historical diagnostic JSON remains readable; new seed reports record explicit acceptance rather than instruction equality.

The [learning diagnostics](../bug-fix/2026-09-09-learning-decision-diagnostics.md) retain sanitized decision details. The [measured application](../feature/2026-09-09-measured-learning-application.md) decision still owns application and rollback; content-review-only mode is unchanged. Tests use the production turn projection and measured suite builder. Deterministic optimizer scoring conveniences compile only for tests; production scoring belongs to the fixed runner.

## Alternatives considered

**Keep full analysis as a review response.** Its summary, observations and exact instruction echo are not needed to decide whether an immutable proposal is supported.

**Merge the review calls or remove their independent checks.** Source-backed seed admission and measured-winner application occur at different stages with different evidence. Sharing their response format does not make either gate redundant.

## Consequences

Reviewing devices need the matching response protocol. Missing or malformed fields fail closed. Fixed-review changes invalidate the optimizer analyst fingerprint. Existing independent invocations, source checks, recurrence eligibility, content-only mode and measured application consent remain separate contracts. This consolidation does not claim better model judgment.
