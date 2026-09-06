# Agent Note: Keep file saves retryable

Status: implemented

## Problem

A failed save replaced the editor with an error screen, leaving its draft
inaccessible. The CodeMirror save keymap captured the initial saved content,
so saving an undo back to that content could silently leave the disk unchanged.

## Decision

`FileEditor` keeps loaded content mounted and displays save errors alongside
it. The existing Save action retries the current draft. Its keymap reads the
current save callback through a persistent ref, sharing the button's dirty and
in-flight checks without rebuilding the editor or discarding undo history.
Responses from a cancelled file load cannot replace the current buffer.

## Alternatives considered

**Rebuild CodeMirror whenever the callback changes.** Rejected because saving
would reset the editing state and undo history for a callback update.

**Reload after a failed save.** Rejected because disk content cannot recover
the unsaved draft. Keeping the editor mounted makes recovery possible without
a second draft store.

## Consequences

The file-editor browser regressions use an owned workspace and assert disk
content after save, undo and retry. A directory substituted for the target file
causes a real write failure. [External-write conflict detection](2026-09-05-editor-external-conflicts.md)
owns the save precondition and explicit conflict choices. These decisions do
not preserve drafts when their editor is unmounted.
