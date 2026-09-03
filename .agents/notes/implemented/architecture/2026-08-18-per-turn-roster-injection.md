# Agent Note: Per-turn roster injection for channel task assignees

Status: implemented

Formerly ADR-004.

## Problem

The channel kanban feature relies on agents creating tasks with valid assignees. Local testing showed an operator agent assigning tasks to roles (`frontend-engineer`, `devops-engineer`) that were not provisioned in the group: agents had no reliable picture of the current roster.

A roster source must (a) reflect membership and `principal.channel_visibility` changes mid-conversation, (b) be uniform across all supported local CLI drivers, and (c) be authoritative: agents must not fall back to template role slots or stale workspace files.

## Decision

Every `[choruz-incoming]` envelope carries a `roster:` field listing the current valid visible assignees for the conversation, recomputed at delivery time by the pipeline. The canonical driver templates (`services/choruz-pipeline/src/instructions.rs` and the web templates it composes) and the AI Manager instructions document the envelope field as the single roster source. No team file exists in agent workspaces; `TASKS.md` boards are named in the bootstrap only to say they are deprecated.

## Alternatives considered

- **Workspace file** (the pipeline writes and refreshes a team JSON file in each agent workspace; the agent reads it on demand): rejected because a file goes stale between refreshes, needs watch, eviction and cleanup logic, and was the exact failure path seen in testing.
- **Tool or API on demand** (the agent queries a runtime endpoint or skill): rejected because it requires per-driver tooling to stay uniform across Claude Code, Codex and Gemini CLIs, and an agent that forgets to ask still guesses.

## Consequences

- The roster is always fresh; there is no file to be stale.
- The per-turn payload grows by one structured line, acceptable for MVP-sized groups.
- All supported local CLI drivers receive the same envelope shape, so behaviour is uniform without per-driver tooling.
- Future expansion (role hints, skill tags) is additive on the same envelope.
- Roster correctness is an envelope-shape test in the pipeline, not a file-content test in each workspace.
- How the change reached workspaces provisioned earlier is owned by [versioned bootstrap refresh](../feature/2026-08-18-versioned-bootstrap-refresh.md).
