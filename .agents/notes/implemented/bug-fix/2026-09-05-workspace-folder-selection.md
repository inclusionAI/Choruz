# Agent Note: Keep workspace folder selection recoverable

Status: implemented

## Problem

A Company without a folder had no picker entry because its only trigger lived inside the conditional file tree. The picker also ignored edited path text on confirmation, allowing a stale current directory or highlighted entry to be submitted.

## Decision

The sidebar provides a folder-selection button for the active Company when its folder is unset. Confirmation resolves the displayed selection through the existing target-aware directory-listing operation before updating the Company. Editing a path clears the stale entry selection; invalid directories keep the picker open and leave the saved folder unchanged. Path editing waits for directory loading so a pending listing cannot overwrite a newly entered path.

## Alternatives considered

**Require Enter before confirmation.** This preserves an avoidable second interaction and leaves pasted paths easy to misunderstand. Confirmation can validate the path through the same filesystem owner used for browsing.

**Accept typed paths without checking them.** That could persist nonexistent or inaccessible directories and bypass the selected device's filesystem boundary.

## Consequences

Confirmation performs one directory lookup on the selected target. The Company update and its error handling retain their existing owner. Browser regressions own their Company and directories, use real controls and filesystem routes, and re-read the saved folder after removal, invalid selection and successful selection.
