---
name: choruz-prose-standard
description: Use when writing, reviewing, restoring, trimming, or auditing prose in this repository, including deciding where documentation or comments are required across Markdown, Rust doc comments, JSDoc, code and test comments, agent instruction templates, prompts, diagnostics, and CLI or UI strings.
---

# Choruz prose standard

Write enough to preserve the contract, then remove reasoning transcripts, repetition and decoration. A contract is an obligation, invariant, precondition, postcondition or compatibility promise that a caller, callee, producer or consumer relies on. This skill owns editorial judgement and required prose coverage; [choruz-doc](../choruz-doc/SKILL.md) owns placement and validation. It is guidance, not a script.

Comments describe non-obvious contracts or rationale that code cannot express; they do not restate what code already implies.

## Inputs and exclusions

Require an explicit scope. If it is missing, report the required input and stop; do not infer a repository-wide scope.

Review and audit tasks report findings without editing; explicitly requested write, fix or trim tasks apply clear changes.

Always exclude from discovery, review and edits: `services/choruz-pipeline/src/instructions_fixtures/` (frozen compatibility fixtures), generated build output, `node_modules/`, `target/`, and `migrations/` files listed in `scripts/historical-migrations.sha256` (checksum-frozen). Inspect an excluded target only to understand an inbound citation.

Treat generated artifacts and recorded expectations as derivative. Trace every consumer before editing: the doc comment on a template may also be model-visible text that a test pins. Edit the owning source first, then regenerate or re-record every derivative.

## Preserve the complete proposition

Before editing, identify every proposition in the passage. Preserve each relevant actor and action; condition, timing and ordering; modality (must, may, never); negative guarantee and exception; ownership, side effect, failure mode and consequence.

Remove adjectives, repetition and narration only when every factual clause survives and the result is clearer. A smaller word count alone is not an improvement.

Keep a complete local contract at the point of use; link the owning document for architecture, rationale, algorithms, history or extended examples. One explanation has one home; essential contract facts may repeat locally.

## Required coverage by prose location

This is not a one-way shortening pass. Add or restore prose when code, types and structure do not communicate a required contract below.

- **Rust public items:** document caller-visible error variants, side effects, ownership, cancellation, ordering guarantees (`server_seq`, idempotency) and durability. A `pub fn` with a non-obvious contract has a doc comment; an obvious getter does not.
- **TypeScript exports (`apps/web/lib`, SDKs):** JSDoc for return distinctions, thrown errors, side effects on caches or `localStorage`, and timing (debounce, optimistic state).
- **Internal comments:** orient non-local structure and obviously complicated local structure: invariants, race ordering, ownership, security and workspace boundaries, surprising failure behaviour. Delete control-flow narration and code restatement.
- **Module comments:** state the module's role, dependencies, responsibilities and non-obvious architecture choices, linking the choice's note.
- **Tests:** explain only non-obvious test design: why a fixture, an indirect observation, a parallel-safety measure or a real entry path is necessary. Delete walkthroughs and inventories.
- **Agent instruction templates and prompts** (`services/choruz-pipeline/src/instructions.rs`, `agent-templates`, `CLAUDE.md`): wording is behaviour. Change it with the instruction tests and fixtures, and state the agent-behaviour risk in the PR.
- **READMEs and `docs/`:** the consumer contract: configuration, semantics, failures, limitations, extension points. Keep durable gaps and maintainer traps, not cleanup inventories.
- **Agent Notes:** retain unique rationale, mechanisms, alternatives, consequences and named coverage gaps. Implemented notes state shipped reality in the present tense.
- **Skills and agent instructions:** behavioural guardrails and explicit scope limits ("guidance, not a checklist"). Keep the workflow concise and link its source of truth.
- **Diagnostics and error messages:** name the failing subject or path, the violated rule and the correction when non-obvious. Remove internal execution narration.
- **UI strings:** treat text, accessibility names, tooltips and placeholders together; keep a change visible in the affected e2e or component test.

Preserve searchable mechanism names and meaningful modal, temporal or negative emphasis. Normalise decorative emphasis only.

## Workflow

1. Confirm the scope, the branch or PR base, and the applicable `AGENTS.md`.
2. Read the owning code or document before judging a passage.
3. Inspect the requested scope, not only the largest files. Use `git grep` and word counts to find candidates, then judge passages semantically.
4. Classify each candidate as keep, add, trim, restore, restructure or defer. Apply clear changes only when the task authorises edits; do not manufacture edits to satisfy a deletion target.
5. Update the owner before derivative artifacts. Re-check analogous passages after learning a new rule.
6. Run the narrow relevant checks (`cargo doc` warnings for Rust doc comments, `pnpm web:check`, the instruction tests for templates, `python3 scripts/verify_agent_notes.py` for notes) and `git diff --check`.
7. Report the inspected scope, clear changes, deliberate keeps, deferred cases and checks actually run.

## Borderline decisions

A case is borderline only when at least two versions satisfy the complete-proposition rule but trade accepted principles. Apply clear edits when authorised and report genuine borderline cases without asking; do not weaken a proposition to make progress. When the user wants to calibrate, present two or three viable versions, recommend one, and state the factual difference.
