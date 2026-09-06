# Agent Note: Require the observed file content before saving

Status: implemented

## Problem

An editor draft can outlive an Agent's disk edit. Unconditional writes replace
that edit without giving the user a choice.

## Decision

The filesystem write route requires `original_content`, the text FileEditor
already retains. The authenticated gateway reads the canonical target through
the same bounded text reader as file loading. A mismatch returns 409 and the
current disk content without writing. Local and remote saves use this owner.

The editor retains its draft on conflict. Reload adopts the returned disk
snapshot; Overwrite submits the draft with that snapshot as its precondition.
A further external edit therefore conflicts again. Both actions are explicit;
ordinary Save never silently retries with a refreshed precondition.

## Alternatives considered

**A separate revision store or file watcher.** Rejected because the caller
already retains the original text and the write owner can compare it directly.
A watcher cannot replace the save-time check.

**Unconditional force-write after confirmation.** Rejected because another
external edit between the conflict and confirmation would be lost silently.

## Consequences

The existing-file save contract requires a precondition and rejects text above
the editor's 1 MiB limit. There is no product caller that creates files through
this route. The check detects preceding writes, not arbitrary external writes
racing after comparison; it is not a filesystem-wide atomic compare-and-swap.
Reload adopts a snapshot, not a live disk subscription.

[Retryable saves](2026-09-05-editor-save-recovery.md) retain their independent
keymap, cancelled-load and failed-save guarantees. This decision does not
preserve drafts after their editor is unmounted.
