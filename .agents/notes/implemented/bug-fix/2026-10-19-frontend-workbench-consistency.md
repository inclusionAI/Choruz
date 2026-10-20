# Agent Note: Keep workbench harnesses and editor appearance consistent

Status: implemented

## Problem

A task composer with its own driver allowlist hides installed default harnesses. A model identifier retained after switching harnesses can belong to the wrong CLI. An editor that combines a light background with dark-theme token colours makes ordinary source code difficult to read.

## Decision

The default task composer derives its local terminal choices from the driver registry's default creatable agents and intersects them with installed availability. Plugin configuration and account selection remain in advanced setup. Changing the harness clears the model field rather than sending the previous harness's identifier to another CLI.

`FileEditor` uses the resolved application theme for both CodeMirror chrome and syntax highlighting. A CodeMirror compartment reconfigures appearance without replacing the buffer or undo history. The affected representative source tokens and line numbers retain at least 4.5:1 contrast in both themes.

## Alternatives considered

**Add Muse to the composer's hard-coded list.** Another driver list can drift again. The registry already owns default harness availability.

**Keep the model when switching harnesses.** Exact model identifiers are provider-specific. Clearing the field chooses the selected harness's default instead of implying compatibility.

**Rebuild the editor when the theme changes.** This loses undo state. Appearance is an independent compartment rather than a new editing session.

## Consequences

The [task-first workbench](../feature/2026-09-29-task-first-workbench.md) retains its navigation and provisioning decisions. [Retryable file saves](2026-09-05-editor-save-recovery.md) and [workbench state recovery](2026-10-05-workbench-state-recovery.md) retain their independent save and draft guarantees; none is superseded by the presentation change.

The component regression covers a Muse-only installation. Browser scenarios cover model reset across harness changes, rendered code contrast across theme switches and retained undo state. The stale driver-scan scenario excludes account discovery, whose verified model list legitimately replaces a driver scan. Capability fixtures isolate UI state from installed provider binaries; these scenarios do not establish live model availability.
