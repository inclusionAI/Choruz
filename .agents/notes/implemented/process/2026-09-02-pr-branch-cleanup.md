# Agent Note: Delete completed pull request branches

Status: implemented

## Problem

Merged and intentionally cancelled pull requests leave stale source branches on `origin`. Those references obscure active work and make it harder to identify the current implementation. Deleting a branch without checking its dependents can also break a stacked pull request.

## Decision

The `choruz-pr` skill treats source-branch cleanup as the final step of a terminal pull request. The required CI is the sole merge gate; review bots are advisory unless a repository rule makes one required. After a merge or an intentional cancellation, the skill verifies the PR state and checks for open PRs based on the source branch. It deletes the remote branch only when no dependent remains, then removes the local branch only when no worktree has it checked out.

## Alternatives considered

**Keep every branch indefinitely.** Rejected because stale refs conceal the set of active branches.

**Delete immediately after any close event.** Rejected because a failed check is not abandonment and stacked PRs can still depend on the branch.

**Rely only on GitHub automatic deletion.** Rejected because repository settings can vary and automatic deletion does not cover intentionally cancelled pull requests.

## Consequences

Merged history remains available through the pull request and squash commit. A branch remains in place while it has open dependents or active worktrees. Cleanup failures are reported and resolved rather than being treated as successful deletion.
