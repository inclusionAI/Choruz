# Agent Note: Versioned bootstrap refresh for existing agent workspaces

Status: implemented

Formerly ADR-006.

## Problem

Channel-task guidance originally shipped in three instruction sources: web agent templates, AI Manager instructions, and an independent Rust-side minimal `CLAUDE.md`. They drifted because the web templates and the pipeline fallback evolved separately. [Per-turn roster injection](../architecture/2026-08-18-per-turn-roster-injection.md) removed the team file and `TASKS.md` from the bootstrap and [the board receipt](2026-08-18-board-tasks-created-receipt.md) changed role contracts; without a refresh path, every workspace provisioned before those changes kept following the old instructions and the kanban rollout was silently partial.

## Decision

`services/choruz-pipeline/src/instructions.rs` composes the canonical web driver shells, the core protocol and the standard extension fragments directly, so web provisioning and the pipeline render the same instructions. Bootstrapper-created files carry a version header and delimit the designed role with `choruz-role:start` / `choruz-role:end`. When a managed file is older than the code version, the pipeline checks that the content before and after the role block still matches the canonical template; only then does it atomically re-render the current template while keeping the role text. The immediately prior managed layouts are frozen as compatibility fixtures under `services/choruz-pipeline/src/instructions_fixtures/` so a template rollout can refresh them without losing role text. Any edit outside the managed layout, or unrelated legacy content, is preserved and surfaced through a warning sidecar instead of overwritten.

Operators have a manual escape hatch: `choruz-pipeline rebootstrap --workspace <path>` (or `--principal <agent-principal-id>`) forces a rewrite regardless of edit state and reports what was replaced. It attempts a best-effort backup first, warns if the backup cannot be written, and still proceeds because the operator asked for replacement.

## Alternatives considered

- **Scope the new instructions to newly provisioned agents only** and require an operator to delete or re-provision agents to pick them up: rejected because it leaves every existing workspace on stale guidance and makes each instruction change a manual rollout.
- **Always overwrite existing bootstrap files on each startup**: rejected because it destroys operator edits without warning.
- **An independent prompt with a prior-body hash ledger** (the first implementation): superseded because the ledger prevented accidental overwrites but produced a separate minimal prompt that drifted from the fuller Claude and Codex provisioning templates; composing the canonical templates removes the second source.

## Consequences

- Web provisioning and pipeline repair cannot drift into separate platform prompts.
- Role-only customisation survives a managed template refresh.
- An edit outside the role block causes preserve-and-warn during automatic refresh; explicit rebootstrap may overwrite it by design.
- The version header remains the refresh point for later instruction churn.
- Tests cover modular composition, clean managed refresh, role preservation, external-edit preservation, and manual rebootstrap.
