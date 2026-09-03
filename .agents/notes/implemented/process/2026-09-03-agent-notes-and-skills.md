# Agent Note: Agent Notes, AGENTS.md and repository skills

Status: implemented

## Problem

Design rationale lived in six ADRs and five RFC drafts under `docs/`, task boards at the repository root, and a vendored copy of a generic skill plugin under `.claude/`. Nothing said when a decision had to be written down, the ADR and RFC formats disagreed, implemented RFCs still read as proposals, and no gate checked any of it. Agents working in the repository had a platform protocol (`CLAUDE.md`) but no engineering rules, so each one re-derived conventions from the tree or from memory. Repeatable work (opening a pull request, fixing a flaky test, reviewing, archiving decisions) had at most one private skill.

## Decision

The repository adopts the deepseek-harness layering, minus its bilingual pairing:

- `AGENTS.md` at the root holds the standing engineering rules, one to three lines each with a link to their home. `CLAUDE.md` imports it (`@AGENTS.md`) so Claude Code reads both; Codex reads `AGENTS.md` natively.
- `.agents/notes/` holds one decision per file at `{lifecycle}/{class}/yyyy-mm-dd-topic.md` with a fixed header block, a per-lifecycle body skeleton and a mandatory `## Alternatives considered`; [README.md](../../README.md) is the contract and `scripts/verify_agent_notes.py` enforces it from CI's Static checks job on any change under `.agents/`. A change under `.agents/` never counts as documentation-only, so the gate always runs.
- `.agents/skills/` holds one `SKILL.md` per repeatable procedure (`choruz-pr`, `choruz-pre-push-checks`, `choruz-ci-test-reliability`, `choruz-code-review`, `choruz-merging-stacked-prs`, `choruz-doc`, `choruz-prose-standard`, `choruz-find-simplifications`, `choruz-archive-agent-notes`). `.claude/skills` is a symlink to it so Claude Code discovers the same files.
- The ADRs became implemented notes; task boards are not retained in the repository.
- `docs/` is tiered with one home per fact ([docs/AGENTS.md](../../../../docs/AGENTS.md)): `docs/architecture.md` is the ordered map, `docs/subsystems/` holds one reference page per subsystem with a fixed skeleton, `docs/defensive-patterns.md` records bug classes that shipped here as rules, and `docs/testing/`, `docs/operations/` and `docs/data-model.md` keep their existing roles.

## Alternatives considered

- **Keep ADRs and RFCs as they were**: rejected because two formats with no lifecycle let implemented designs stay written as proposals, and nothing required a new decision to be recorded at all.
- **Rewrite the four RFCs into present-tense implemented notes during the migration**: rejected for this change because each is several hundred lines of proposal-era plan; a faithful rewrite is one follow-up per feature, and a wrong rewrite would be worse than an honest archive.
- **Adopt DSH's bilingual sidecars (`.zh.md` plus a hash record)**: rejected because the team reads English documentation and the pairing gate would double the cost of every prose edit for no reader.
- **Keep skills only in personal `~/.claude/skills`**: rejected because a skill nobody else can see is not a repository procedure, and Codex agents never see it at all.
- **Put the skills under `.claude/skills` as the canonical location**: rejected in favour of `.agents/skills` with a symlink, so the procedures are not tied to one agent product.
- **A TypeScript gate like DSH's**: rejected because the repository's CI selectors and their tests are already Python under `.github/scripts`, and the verifier needs nothing beyond the standard library.

## Consequences

- Every non-trivial pull request now carries a note; the template asks for it and reviewers block on its absence. That is deliberate friction.
- A notes-only pull request runs one Static checks job, about a minute; a documentation-only one still runs nothing.
- The current design of channel tasks, threads, hybrid routing and native DM history has no present-tense note until someone writes one.
- `AGENTS.md`, `CONTRIBUTING.md` and the skills overlap in places by design: `AGENTS.md` is the standing order, `CONTRIBUTING.md` the human entry point, the skill the procedure.
