---
name: choruz-code-review
description: Use when reviewing a pull request in this repository; orients the reviewer to Choruz's standards (AGENTS.md conventions, the PR test policy, Agent Notes, the CI gates) and the review-specific checks that a green CI cannot show.
---

# Reviewing a Choruz PR

**This skill is guidance, not a complete checklist.** Fetch the PR's live base and exact head, run `bash .agents/skills/choruz-pr/pr-plan.sh <base>` on the head to see which surfaces the diff reaches, then read the diff and enough surrounding code to understand the design. Prioritise correctness, data isolation, lifecycle, security and broken required behaviour over style; a short review with one substantiated blocker is better than a list of nits.

## Sources of truth

- [AGENTS.md](../../../AGENTS.md): standing repository rules.
- [docs/testing/pr-test-policy.md](../../../docs/testing/pr-test-policy.md): what tests each change type must add and what CI runs for it.
- [docs/architecture.md](../../../docs/architecture.md) and [docs/data-model.md](../../../docs/data-model.md): how the system and its tables fit together.
- [Agent Notes](../../notes/README.md): design rationale. Treat disagreement with a note as a design discussion, not an automatic veto.
- [choruz-prose-standard](../choruz-prose-standard/SKILL.md): required coverage and editorial judgement for comments, docs, prompts and visible strings.
- [choruz-ci-test-reliability](../choruz-ci-test-reliability/SKILL.md): isolation and regression-proof rules for parallel, resource-owning or flaky tests.

## Blocking requirements

1. **The Agent Note exists.** A non-trivial change (behaviour, architecture, shared contract, process, format) links a note it adds or updates, in the same PR; a proposed note being implemented is moved to `implemented/` and rewritten in the present tense in the same diff. "none: mechanical" must be true.
2. **Tests match the declared type.** A `bugfix` has a regression test that fails without the fix; a `feature` has unit or integration coverage and an e2e spec for a new user-facing flow; a `database` change has a new migration and a smoke run. Reject a type chosen to avoid tests.
3. **Workspace scoping.** Every new query or command in `crates/choruz-application` filters by `workspace_id`; a new table carrying user data has the column and the index.
4. **Migrations are append-only.** No edit to a file listed in `scripts/historical-migrations.sha256`; a new `V0NN__name.sql` with a matching `docs/data-model.md` update.
5. **Contracts first.** A wire, database or configuration change updates `openapi/` or the migration in the same diff.
6. **Docs match the code.** A moved file, renamed key, changed default or new command updates the document that names it (`docs/`, README, the web docs pages under `apps/web/app/docs`) in the same diff. Comments state non-obvious contracts; flag narration, review history and duplicated rationale.
7. **Model-visible text is tested.** A change to what agents receive (the `[choruz-incoming]` envelope, instruction templates in `services/choruz-pipeline/src/instructions.rs` and `agent-templates`, bootstrap fixtures) updates the instruction tests and fixtures, and states the agent-behaviour risk.
8. **Required evidence exists.** The template's "Ran locally" names the commands the diff needed (see [choruz-pre-push-checks](../choruz-pre-push-checks/SKILL.md)); `CI (linux) required` is green on the exact head.

## Manual checks

- **Intent and interface contracts:** trace both sides of every changed interface (gateway handler and web client, pipeline stage and store, connector and bridge). Confirm errors, cancellation, idempotency and ownership match the PR and any note.
- **Message path:** for anything on the send, outbox, sync-feed or fanout path, check idempotency keys, `server_seq` ordering, optimistic-message reconciliation in the web client, and that every visible message is reconstructable from the store.
- **Lifecycle and concurrency:** for spawned agents, terminals, the outbox watcher, cron and the supervisor, check races before publication, cancellation during awaits, independent error reporting, callback containment, and disposal to quiescence.
- **Enforcement:** follow every permission denial and workspace check to the operation that executes it; exercise direct and alternate callers (API, connector, CLI, bridge) that could bypass the UI.
- **Scope and necessity:** map each abstraction, option, compatibility path and defensive copy to a current consumer. Challenge speculative generality and unrelated features.
- **Real entry path:** e2e tests exercise the shipped web app against the real API and pipeline through `infra/host/web_e2e.sh`, not a mocked route where the behaviour under test lives server-side.
- **Test strength:** assertions fail on the intended regression and verify external state (rows, events, files, exits) rather than restating the implementation. A test that owns no data is a future flake (apply [choruz-ci-test-reliability](../choruz-ci-test-reliability/SKILL.md)).
- **Selector rules:** a new e2e spec for a new feature area gets a rule in `.github/scripts/select_e2e_specs.py`, or CI will never select it.
- **Seams touched match the diff:** for a `feature`, `api` / `database` or `security` / `auth` PR, read the template's "Seams touched" section against [docs/adding-a-feature.md](../../../docs/adding-a-feature.md). A new route without a `require_*` helper, a new table without `workspace_id`, a new spec without a selector rule, or a ticked seam the diff does not contain is a finding.
- **Implemented notes match shipped reality:** paths, names and mechanisms in the note agree with the implementation.

## Reporting findings

State the defect, location, impact and evidence. Place a localised defect inline on the tightest relevant diff range; use a PR-level comment for cross-cutting architecture, scope, or review-wide synthesis. Separate blockers from suggestions and omit issues a green gate already enforces. When receiving review, verify each claim and fix or rebut it on technical grounds without performative agreement.
