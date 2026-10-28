# Agent Note: Saving unchanged community permissions keeps the learning generation

Status: implemented

## Problem

`configure_behavior_community` incremented the policy generation and cleared leases on every save, including a save of the permissions already stored. The learning dialog saves community permissions as part of its normal flow. Once background workers began waking immediately ([background-work wakeups](../architecture/2026-10-28-background-work-wakeups.md)), a check could commit a reviewed candidate before that save. The save then moved the policy to a new generation; automatic optimization only queues candidates of the current generation, so the reviewed candidate was never evaluated or applied, and no error was shown. The remote learning acceptance test failed in CI with an empty evaluation list for this reason.

## Decision

`configure_behavior_community` locks the policy row, compares the stored settings with the requested ones as `CommunitySettings`, and returns without writing when they are equal. Only a changed consent increments the generation, clears learning leases and releases behavior-event leases, so revoking or granting a permission still fences generation-scoped work.

## Alternatives considered

**Re-queue candidates from the previous generation after a community save.** A real consent change is meant to fence work produced under the old consent; carrying candidates across it would defeat that.

**Have the dialog skip saving unchanged permissions.** Other clients, scripts and retries can still send identical settings; the owner of the fence must decide what counts as a change.

## Consequences

Re-saving the same permissions is a no-op. Comparing parsed settings, not JSON text, treats the stored default and an explicit default as equal. A regression test commits a reviewed candidate, re-saves the default permissions and requires the same generation and a pending automatic optimization, then changes a permission and requires the generation to advance; the previous behavior fails it.
