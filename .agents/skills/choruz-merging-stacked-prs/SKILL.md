---
name: choruz-merging-stacked-prs
description: Use when landing a chain of dependent pull requests (A ← B ← C, each based on the one below) onto main, merging a PR whose base is another open PR's branch, or whenever a request mentions "stacked PRs", "PR stack", "dependent PRs", or merging several related PRs in sequence.
---

# Landing a Choruz PR stack

A stack is a chain of pull requests where each upper PR's base is the head branch of the one below. Land it bottom-up, one green squash at a time, and let CI re-run against `main` before every merge. Never reproduce a stack by rewriting someone else's branch.

## Establish the chain

Fetch current metadata for every PR rather than trusting branch names or an earlier report:

```sh
gh pr view <pr> --json number,author,baseRefName,headRefName,headRefOid,state,isDraft,mergeStateStatus,statusCheckRollup
```

The bottom PR targets `main`; each higher PR targets the head branch immediately below. Any other shape (two PRs targeting the same branch, a cycle, a cross-fork head) needs the user's direction before any mutation. If GitHub's native stack feature (`gh stack`) is available and the chain is already linked, prefer `gh stack merge` for the whole range and skip to [Verify the landed state](#verify-the-landed-state); otherwise follow the manual sequence below.

## Land one layer

For the bottom PR:

1. Require it open, non-draft, and `CI (linux) required` green on its exact current head. A green upper layer proves nothing about its dependencies.
2. Squash-merge it (the repository convention). Do not delete the branch yet: the next layer still bases on it.

For the next PR up:

1. Retarget its base to `main`: `gh pr edit <pr> --base main`.
2. Bring `main` in. Its branch already contains the lower branch's commits, and `main` now contains their squash; a merge resolves cleanly when nothing else touched the same lines: `git merge --no-edit origin/main`, then push. On a branch you created, a rebase is equally acceptable; on someone else's branch, only a merge commit.
3. The push re-triggers CI against the real base. Wait for `CI (linux) required` on the new head, then squash-merge.

Repeat until the requested range has landed. "Land the stack" means every layer; a partial landing names an explicit boundary PR and stops there.

## Do not

- Merge an upper layer before its base has merged, or while its base is still another PR's branch.
- Force-push, rebase, or amend a branch you did not create.
- Push an empty commit or close and reopen a PR to re-run CI; a base change plus `git merge origin/main` is the correct trigger.
- Bypass a red or missing `CI (linux) required` on any layer.

## Verify the landed state

Wait for every selected PR to report `MERGED`:

```sh
gh pr view <pr> --json number,state,mergedAt,mergeCommit,baseRefName,headRefName
```

Delete branches only in a final pass after the corresponding PRs report `MERGED` and no open PR still uses the branch as a base:

```sh
gh pr list --state open --base <branch> --json number --jq length   # must print 0
```

## Checklist

- [ ] Live PR bases and heads establish one bottom-to-top chain.
- [ ] Each layer merged only when green on its exact head, bottom first.
- [ ] Each upper layer retargeted to `main` and refreshed with `main` before its own CI run.
- [ ] No branch rewritten that someone else created.
- [ ] Every selected PR reports `MERGED`; branches deleted only after zero-dependent verification.
