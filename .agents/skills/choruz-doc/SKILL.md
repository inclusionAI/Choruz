---
name: choruz-doc
description: Create, restructure, review, audit, or migrate Choruz Markdown documentation (docs/, README, AGENTS.md, the in-app docs pages) using one owner per fact, tier placement, executed-operation fact-checking, current-state prose, and the repository validators. Use for new or revised docs, docs-tree organisation, and documentation-quality audits.
---

# Choruz documentation

## Summary

Make every page findable, newcomer-readable, and exact enough for agents and maintainers, with one owner per fact. Apply `AGENTS.md` and the gates first, then this workflow for placement, progressive detail, fact-checking, and validation. Design rationale lives in [Agent Notes](../../notes/README.md); this skill owns everything else written for a human or agent reader.

## The tiers: one home per fact

| Tier | Home | Holds | Not |
|---|---|---|---|
| Standing orders | `AGENTS.md` | One to three lines per rule, linking its home | Rationale, procedures |
| Platform protocol | `CLAUDE.md` | How an agent talks to humans and agents on Choruz | Engineering rules |
| Architecture | `docs/architecture.md` | The ordered map of how the system works today | Per-crate detail, history, status |
| Data model | `docs/data-model.md` | Tables, ownership, `workspace_id`, sequences | Migration narration |
| Operations | `docs/operations/` | Install, run, deploy, back up, SLO, runbook | Design rationale |
| Testing | `docs/testing/` | PR types, what CI runs, e2e conventions | Test walkthroughs |
| Decisions | `.agents/notes/` | Problem, decision, alternatives, consequences | Current-state reference |
| Contributor entry | `CONTRIBUTING.md`, `README.md` | Setup, commands, layout, links into the tiers | Duplicated rules |
| User docs | `apps/web/app/docs/` | The in-app documentation site | Engineering process |
| Package contracts | a package's `README.md`, doc comments | Config, semantics, failures, limitations | Cross-package narrative |

Placement: bugs and incidents to a note or a runbook; rationale to notes; procedures to operations; type and table definitions to the data model; package contracts to the package README; standing orders to `AGENTS.md` with a link.

## Workflow

1. Read `AGENTS.md`, the target page, its source or tests, and the page that links to it.
2. Classify the page by one job and reader: setup tutorial, operator procedure, architecture overview, reference, decision record, user guide.
3. Place it at its nearest owner from the table. Keep package contracts beside the package; use `docs/` for cross-package material.
4. Define the reader's starting state, observable outcome, likely failure, recovery path and next depth before writing details.
5. Write the common path briefly and link exhaustive truth one hop away. Each section opens with a short orienting paragraph.
6. Verify every claim against code, tests, migrations, scripts, or `package.json`, and run the operations the page instructs (below).
7. Run the validators, then re-read the whole diff once for correctness and once for brevity.

## Fact-check procedure: test, do not assume

The only admissible evidence for an operation claim is having run it.

1. **Run every claimed command against the current checkout**, exactly as the page will show it; write down only what you observed, including warnings and failure modes. `pnpm` scripts come from `package.json`; a documented flag must exist in the binary's `--help`.
2. **Delete what you could not reproduce.** Never carry a command, field, default or behaviour from memory or from a neighbouring page. When a claim fails to reproduce, fix the claim.
3. **Check old pages against `origin/main`** before revising them; a stale statement on `main` is still wrong and is corrected against the code, not against the old prose.
4. **Environment variables** are named from their reading site (`git grep CHORUZ_` over `infra/`, `services/`, `apps/`), with the default the code applies.

## Voice rules

- **Summary says what the subject does** for the reader (outcomes, when to choose it, main cost), never its role or internal identity.
- **Reference sections explain, never enumerate** what a generated or source artifact already lists; link the source for exact detail.
- **Current state only.** No "previously", "now", "no longer", PR numbers or migration narration outside notes' `Consequences`.
- **Controlled technical English:** one actor and one main action per sentence where ambiguity could change behaviour; one term per concept; direct verbs; preserve modality and exceptions. Chinese is welcome in `README.md` and user-facing pages where the existing page is Chinese; keep a page in one language.
- **Link by relative path**, never by bare filename or note number.

## Quality criteria

- **Brief:** the common path contains only facts needed for its outcome.
- **Intuitive:** prerequisites precede dependent concepts; one next action is obvious; headings use terms readers search for.
- **Friendly:** readers can recognise success, understand risk before acting, and recover from likely failure.
- **Accurate:** each durable claim has one owner and a verification path.
- **Agent-readable:** stable headings, anchors, terminology and ownership support targeted retrieval.
- **Newcomer-complete:** an engineer with no repository context can reconstruct the relevant architecture through three to five linked pages.

## Audit the corpus

Hunt the slop list with the cheapest probes first: `git grep` distinctive phrases to find duplicated rules; find hand-written inventories that a script or the tree already owns; find migration plans and future-tense spec language in implemented notes; find "previously/now/no longer". Keep every load-bearing rule, preferably as one to three lines plus a link to its rationale.

## Validation

- Links: every relative link resolves; check the targets or open the page in the rendered preview.
- Agent Notes: `python3 scripts/verify_agent_notes.py`.
- Commands: re-run every command the page instructs before merging a claim about it.
- Web docs pages (`apps/web/app/docs`): `pnpm web:check`, and the docs e2e spec when the page structure changed.
- A docs-only pull request runs no test job and merges in seconds; do not mix a config or code change into it.

Use [choruz-prose-standard](../choruz-prose-standard/SKILL.md) for sentence-level judgement.
