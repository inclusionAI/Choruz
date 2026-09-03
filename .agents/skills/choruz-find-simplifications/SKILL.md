---
name: choruz-find-simplifications
description: Use when working in this repository to find non-obvious simplification candidates, remove redundant comments or implementation-heavy documentation, write proposed Agent Notes or inline TODO/FIXME/XXX notes, audit or coalesce superseded Agent Notes, or fold worthwhile simplification ideas from another branch; especially for dead, duplicated, speculative, over-built, added-then-removed, or hand-rolled-where-a-crate-exists surfaces.
---

# Finding Choruz simplifications

Turn a broad "find things to simplify" request into evidence-backed Agent Notes that remove or collapse existing surface area. This is guidance, not a checklist: follow the code, keep judgement active, and prefer a few well-proven candidates over a pile of thin guesses.

## Start with repository context

- Read [AGENTS.md](../../../AGENTS.md), especially the pre-release stance: no compatibility shims, delete the old path.
- Skim [docs/architecture.md](../../../docs/architecture.md) before judging anything under `crates/`, `services/` or `services/choruz-pipeline`; a simplification that fights the stage model (router, executor, writer, fanout) or the sync feed needs extra evidence.
- Use the [Agent Note tree](../../notes/README.md) to learn intentional architecture. A recorded decision is not a veto, but a candidate that collapses one must beat its recorded rationale, not just cite this skill.
- Treat the multiple terminal drivers (Claude Code, Codex, Gemini) and the connector, bridge and remote-control transports as intentional by default; removing an unused method inside one of them can still be valid.

## What counts as a strong candidate

- A handler, endpoint, event, config knob, helper, crate, table column or test fixture has no production consumer.
- Tests or docs are the only consumers, and the behaviour they pin is not load-bearing.
- Two representations mirror the same fact (a cached count and its source rows, a client-side copy of server state that the sync feed already carries).
- A shared crate exposes methods every service must support but no service calls.
- A feature implements speculative product generality with no owner.
- A hand-rolled parser, retry loop, path matcher or serializer duplicates a maintained crate or npm package the workspace already depends on, and the swap would delete the implementation plus its dedicated tests.
- Dead paths left by a retired surface (the desktop app, the maildir protocol's predecessors, renamed features) that still compile because nothing removed them.

Thin candidates are not enough for a note: one typo, one unused import, or "this looks complex" without call-site proof. Fix those directly or leave a TODO.

## Survey broadly

Use parallel subagents when the user asks for breadth, one domain each, evidence required. Useful domains: the message path (gateway handlers, outbox, sync feed, fanout); the pipeline (router, executor, writer, cron, watcher); the web client (hooks under `apps/web/lib`, components, the docs pages); runtime and host (`choruz-supervisor`, supervisor, connector, bridge); scripts, infra and CI. Start with the largest production files (`git ls-files | xargs wc -l | sort -rn`); the biggest deltas hide the duplicated lifecycle machinery.

## Prove or reject each candidate

Classify consumers before writing:

- **Production corpus:** `crates/*/src`, `services/*/src`, `apps/*/src`, `apps/web/{app,components,lib,hooks}`, `infra/`, `migrations/`.
- **Non-production corpus:** tests, `docs/`, notes, fixtures, snapshots, comments.
- **Ambiguous corpus:** `scripts/` and the e2e harness, which may be product smoke paths.

Use `git grep` first: the exact symbol, the route, the event name, the table or column, the config key, the env var. Then read the call sites, the trait impls, dynamic dispatch (driver type strings, event type strings), and the SQL.

Reject or downgrade when a production caller exists and the change would be a feature decision; when an implemented note justifies the API and the new evidence does not beat it; when the removal forces unrelated churn without reducing the surface; or when the idea is correct but tiny (use a TODO with the urgency tags below).

## Simplify prose with the code

Comments and documentation are maintained surface area. Delete comments that restate code or explain behaviour owned elsewhere; keep required local contracts. Apply [choruz-prose-standard](../choruz-prose-standard/SKILL.md).

## Coalesce superseded Agent Notes

When a simplification makes an owning note obsolete, follow [choruz-archive-agent-notes](../choruz-archive-agent-notes/SKILL.md) for retention judgement and mechanics. Preserve every unique rationale, alternative, consequence and reintroduction condition in the current owner; repair inbound links; delete or archive per the README's consolidation rule. Do not expand every code survey into a repository-wide note audit.

## Write the Agent Note

One file per durable proposal at `.agents/notes/proposed/<class>/yyyy-mm-dd-topic.md` (class `simplification` for a removal):

- `# Agent Note: <action-oriented title>` / `Status: proposed`
- `## Problem`: name the current surface, cite the files, state the consumer evidence, separating production callers from tests and docs.
- `## Proposal`: exactly what to remove, fold, or rehome, including tests, docs, migrations and selector rules.
- `## Alternatives considered`: the strongest case for keeping it, made legible.
- `## Acceptance criteria`: the observable end state and the gates that prove it.
- `## Risks`: behaviour changes, future product wants, and why the trade-off is still reasonable.

## Inline TODO notes

Use `FIXME` (blocks a release), `TODO` (soon) and `XXX` (someday) only for small, local cleanups that are clearly useful but not durable design decisions. Name the smell with a stable tag, say why it is safe to revisit, and what action would simplify it.

## Validation and PR hygiene

For note-only work run `python3 scripts/verify_agent_notes.py` and `git diff --check`. For a removal, run what [choruz-pre-push-checks](../choruz-pre-push-checks/SKILL.md) selects for the touched crates and specs, and `git grep` the removed symbols afterwards. In the PR, summarise the areas surveyed, what was intentionally excluded, the candidates added or consolidated, and the checks that passed. Use a draft PR while the survey is still expanding.
