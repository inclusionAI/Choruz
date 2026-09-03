---
name: choruz-archive-agent-notes
description: Use when adding, auditing, pruning, archiving, restoring, or reviewing Agent Notes in this repository; checks every new note for superseded active records, classifies implemented notes by future decision value, deletes rejected notes that no longer prevent a tempting mistake, and applies the frozen archived/{class} rules.
---

# Archive Choruz Agent Notes

Reduce the active decision corpus without erasing history that can still guide work. Judge every note semantically; word count and age are discovery aids, never archive criteria.

## Read the contracts

Read [the Agent Note rules](../../notes/README.md), [the archive instructions](../../notes/archived/AGENTS.md) and [the implemented-folder instructions](../../notes/implemented/AGENTS.md) before classifying. Use current code, migrations, configuration, `docs/`, newer notes and inbound links to establish whether a rationale still owns or constrains anything.

## Check supersession when adding a note

Every new Agent Note triggers a scoped audit of active notes covering the same decision, mechanism, or rejected alternative. Classify each full or partial supersession while writing the new note: archive qualifying implemented notes in the same PR, retain and cross-link partial supersessions or independently useful rationale, reject obsolete proposals, and delete rejected notes that no longer prevent a plausible mistake. Apply the consolidation rule in the README when the new owner absorbs every unique proposition; do not defer a known match to a later corpus audit.

## Classify by future value

- **Implemented, keep active:** retain a note when its rationale, alternatives, negative guarantees, wire or database semantics, ownership boundary, security rule, or reintroduction condition is likely to guide a future change. Length does not matter.
- **Implemented, archive:** archive a note when the shipped decision is complete and its body is unlikely to guide future work: one-off UI chrome, a narrow adapter, a minor closed bug, superseded implementation detail, or process history whose current behaviour is obvious elsewhere.
- **Proposed, never archive:** keep a live proposal active; if it is no longer worth pursuing, reject it with an honest reason on the `Status:` line.
- **Rejected, keep only as a guardrail:** retain a rejection only when the losing proposal remains a tempting, meaningful mistake and the note explains why it loses.
- **Rejected, delete:** delete the note when the rejected idea is obsolete, superseded, or unlikely to prevent re-litigation. Repair or delete inbound links.

Do not archive toward a quota. Inspect every note in scope, classify analogous groups under one principle, use best judgement for close cases, and record genuinely borderline decisions in the PR.

## Calibrated examples

Archive implemented notes such as a closed sidebar-chrome tweak, a one-driver adapter detail whose current code is authoritative, or a completed migration checklist. Keep implemented notes such as [workspace-scoped isolation](../../notes/implemented/architecture/2026-08-18-workspace-scoped-isolation.md) (an ownership and security boundary) or [per-turn roster injection](../../notes/implemented/architecture/2026-08-18-per-turn-roster-injection.md) (it states the negative guarantee that no team file exists). The four RFCs under `archived/feature/` are the reference for what archival looks like: header block added, body untouched.

## Archive one implemented note

1. `git mv` the file from `implemented/<class>/` to `archived/<class>/`; `implemented` is deliberately absent from the archive path.
2. Insert only `Archived: YYYY-MM-DD` immediately below `Status: implemented`. Make no other body edit: do not reformat, update facts, or repair links inside the note.
3. Search for inbound links from active prose (`git grep -n "<filename>"`). Redirect them to the current authority, retarget them to the archived path only when the historical snapshot is intentionally cited, or delete them.
4. Run `python3 scripts/verify_agent_notes.py`.

After the move, never edit, move, or delete the note. Archived notes remain valid link targets but are historical snapshots, not authority for current behaviour.

## Validate and report

Run `python3 scripts/verify_agent_notes.py`, `python3 -m unittest discover scripts/tests`, and `git diff --check`; select any additional evidence through [choruz-pre-push-checks](../choruz-pre-push-checks/SKILL.md).

Report active implemented notes kept, implemented notes archived, rejected notes kept or deleted, proposed notes rejected if any, and every genuinely borderline case with its chosen outcome.
