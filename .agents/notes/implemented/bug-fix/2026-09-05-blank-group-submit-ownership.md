# Agent Note: Blank group submit ownership

Status: implemented

## Problem

The ordinary member-picker group form can submit another create request while its first request is pending. Each accepted request creates a distinct conversation, so double clicking on a slow connection produces duplicate groups.

## Decision

`CreateGroupModal` uses its existing `launchingGroup` state for blank-group submissions as well as template launches. The submit handler rejects reentry, disables submit and draft changes while waiting, and retains the existing modal-close and template-selection lock. Completion or failure releases the state.

## Alternatives considered

**A separate blank-group busy state.** Both paths occupy the same modal and must lock the same navigation; separate state would duplicate ownership.

**Server deduplication by group name.** A name does not identify a creation intent; independently created groups may legitimately share it. Reliable replay after an ambiguous lost response requires an explicit request identity, not a name heuristic.

## Consequences

Repeated clicks during a pending request cannot create another group, and a rejected request leaves the form usable. This is not server-side idempotency for lost responses or a reopened form. The browser regression holds an actual create response and checks the owned conversation count in PostgreSQL, rather than treating a disabled button as sufficient evidence.
