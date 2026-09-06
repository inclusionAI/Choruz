# Agent Note: Scope Explorer state to its workspace

Status: implemented

## Problem

Explorer directory caches can survive a Company switch while their rendered children do not. Returning to the first workspace then suppresses the fetch needed to show its files. Treating failed reads as empty arrays also caches a false success and removes the user's recovery path.

## Decision

The FileTree boundary keys its stateful contents by workspace and root path. Directory reads reject failures; only successful reads enter the loaded set. Failed expansion collapses the affected node so another click retries. Refresh reports failures and provides an explicit retry without discarding the ability to browse recovered files.

## Alternatives considered

**Reset each state field when props change.** A keyed ownership boundary resets the cache, focus and expanded nodes together rather than maintaining a growing list of independent resets.

**Treat failures as empty directories.** An unreadable directory and a successfully read empty directory have different recovery requirements. Only the latter may be marked loaded.

## Consequences

Switching workspaces resets expansion instead of preserving it. Within one workspace, refresh retains expanded paths. The browser regression creates two real workspace directories, switches between them, removes and restores a child directory, and checks the recovered file content through the editor. It does not replace the filesystem route with a mock.
