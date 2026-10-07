# Agent Note: Task-first workbench

Status: implemented

## Problem

Exposing group collaboration, runtime administration and project files together makes starting an ordinary Agent task require understanding the orchestration platform first.

## Decision

Use the existing chat shell as a task-first workbench. Direct conversations form its task list; background collaboration and project files are explicit entries. New-task submission uses the authenticated provisioning route and the structured session command owner. It does not introduce another task database, provider client or runtime.

The first instruction has a stable submission identity so returning to an open task does not repeat it. [Workbench state recovery](../bug-fix/2026-10-05-workbench-state-recovery.md) retains pending instructions through reload until command acceptance. Provisioning retries retain their idempotency key. Advanced setup retains the existing device, account and workspace selection rather than duplicating those controls in the default composer.

## Alternatives considered

**Separate desktop-style application.** This duplicates navigation and runtime integration without improving the task execution contract.

**Hide groups with CSS.** Pinned and archived groups would still leak into task navigation. Filter the navigation projection before section construction instead.

**Delete collaboration.** This removes existing capabilities instead of keeping them available behind an explicit entry.

## Consequences

Task-first navigation changes presentation, not autonomous team orchestration. Claude and Codex keep the structured session contract described in [structured direct conversations](2026-09-06-structured-agent-dm.md). Online groups retain their separate authorization boundary and shared conversation renderer; opening background collaboration exposes them alongside local groups.

[Desktop runtime owner](../architecture/2026-09-29-desktop-runtime-owner.md) supersedes the packaging alternative while retaining this shared workbench and task execution contract.

Browser acceptance traverses provisioning, session commands and approvals with a deterministic external CLI fixture and verifies its filesystem result. This verifies integration, not live provider availability or model quality.
