# Agent Note: Behaviour acceptance evidence

Status: implemented

## Problem

Passing tests can leave an ordinary workflow broken when their assertions accept both success and failure, their fixtures make the missing behaviour invisible, or they bypass the affected production entry. Device and account selection cross process-local path and identity boundaries that a single-environment function test cannot establish. Running more of those tests does not repair the evidence gap.

## Decision

[The test policy](../../../../docs/testing/pr-test-policy.md#behaviour-acceptance-evidence) owns acceptance contracts, safe negative controls, real-entry evidence, result-owner assertions and risk-selected ordinary scenarios. The PR, pre-push, reliability and review skills link that policy at the point each procedure uses it. The [PR template](../../../../.github/PULL_REQUEST_TEMPLATE.md) records the contract, scenario, substitutions, tested scope and unavailable evidence. Command selection belongs to pre-push checks rather than a duplicate full-suite recipe in the policy.

The author checks acceptance evidence before declaring readiness. CI remains the required merge check and review bots remain advisory; the policy does not pretend the aggregator mechanically verifies assertion strength. Live checks use authorised test environments, and an unavailable live dependency is reported separately from deterministic fixture results.

## Alternatives considered

**Run every test for every change.** This increases cost without making weak assertions meaningful. The selected scenario must reject the broken behaviour; broader execution is reserved for genuinely broader effects.

**Require live providers or screenshot baselines for every change.** These tools are useful at their boundaries, but credentials, nondeterministic output and irrelevant visual changes would obstruct unrelated work. The policy selects evidence by the affected contract and supports stable replay or isolated component fixtures where appropriate.

**Add a wording or coverage gate as proof of acceptance.** A template checkbox, assertion-name regex or executed-line percentage cannot prove that the user-visible result is correct. The policy requires semantic inspection and observed negative controls for fixes and guards rather than claiming a mechanical gate solves that problem.

## Consequences

Changes carry more specific evidence, especially across devices, accounts and retained UI state, without requiring every possible combination. Authors must report blocked paths honestly. Existing weak tests are not repaired by this policy change; each affected implementation still needs its own regression and acceptance work.

The [feature seams decision](2026-09-03-feature-seams-checklist.md) remains active because it owns where features integrate, not how assertions prove their behaviour. The [notes and skills decision](2026-09-03-agent-notes-and-skills.md) retains the repository-wide ownership rationale. Neither is superseded or archived.

## Validation

A read-only independent instruction evaluation using the terminal spec and remote provisioning route/tests rejects their weak or bypassed acceptance evidence, retains their limited unit value, exempts a prose-only typo from product scenarios and classifies OAuth without credentials as blocked live evidence. This checks instruction decisions, not product execution. The note verifier and nine repository script tests pass; the script tests emit temporary-directory cleanup warnings. Skill frontmatter and local document links are validated separately.
