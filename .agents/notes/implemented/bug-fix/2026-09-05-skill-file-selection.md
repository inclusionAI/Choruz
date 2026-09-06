# Agent Note: Skill file selection

Status: implemented

## Problem

Installing a local skill requires copying its absolute path outside Choruz, although the application already has directory navigation.

## Decision

The Skills import form opens the existing `FolderPickerModal` with a `.md` file filter. Directory navigation and selection remain the default for other callers. File mode lists directories and matching files, allows selection only from the current listing, and fills the import form without installing until the user clicks Import. The existing authenticated import route reads and validates the selected file.

## Alternatives considered

**A separate file browser.** This duplicates the existing modal, navigation, loading and error behavior. The shared directory listing accepts an optional file flag instead.

**A browser upload input.** It supplies browser file contents rather than the path consumed by the existing workspace import operation.

## Consequences

The original skill file is not modified. Filesystem and import authorization are unchanged; this does not add remote-device skill installation. [Folder selection recovery](2026-09-05-workspace-folder-selection.md) still owns directory validation and failed association updates. Browser tests verify real listing, explicit import and copied content, alongside directory-mode regressions.
