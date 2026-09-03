---
name: choruz-pr
description: Use before opening or completing a pull request in this repository — classify the change, add the tests its type requires, run exactly what CI will run, label it, merge only on a green "CI (linux) required", and clean up completed branches safely.
---

# Opening a pull request

The rule of this repository (docs/testing/pr-test-policy.md): **CI runs what the
changed paths need; the type you declare tells reviewers what tests to expect and
labels can only add gates.** This skill walks a change from "done on a branch" to
"merged" without a person having to babysit it.

## 1. Classify the change

Pick one type from the policy table and keep it in mind for every later step:

| Type | Tests you must add before the PR exists |
|---|---|
| `feature` | Unit/integration tests for the new behaviour; an e2e spec for a new user-facing flow |
| `bugfix` | A regression test that fails without the fix |
| `refactor` | None new; the touched module's existing tests must pass |
| `ui` | Component/unit test when behaviour changes; e2e for a critical path |
| `api` / `database` | Tests for the behaviour change; a migration gets a smoke run |
| `security` / `auth` | Tests for the new rule; e2e regression for login/permission flows |
| `ci` / `build` / `deps` | Policy tests for CI scripts |
| `docs` / `chore` | None |

If the diff spans two types, use the stricter row. Never pick `docs` or
`chore` to avoid tests: CI reads paths, not the type, so it does not help.

For `feature`, `api` / `database` and `security` / `auth`, walk the seams in
[docs/adding-a-feature.md](../../../docs/adding-a-feature.md) before writing
tests, and fill the template's "Seams touched" section from that walk. A seam
left unchecked carries a one-line reason ("no new table", "core route, no
plugin gate"); the reviewer reads the section against the diff.

## 2. Add the minimum tests the row requires

Do this before anything else. A `bugfix` without a failing-then-passing
regression test is not ready. Put e2e coverage next to the feature's spec so
`select_e2e_specs.py` picks it up (add a rule there when you add a spec for a
new feature area).

## Naming new packages and files

Rust packages are `choruz-<role>`: directory equals the package name, the crate ident is `choruz_<role>`. npm packages are `@choruz/<role>`. Libraries live in `crates/`, long-running processes in `services/`, human entry points (`choruz-cli`, `choruz-replay`, `web`) in `apps/`. Files: Rust `snake_case.rs`, web `kebab-case.ts(x)`, tests beside the module (`foo.test.ts`, `tests/<topic>.rs`), migrations `V0NN__name.sql`. A name states a role, never a layer or a generic word.

## 3. Simplify the diff

Run `/simplify` on the branch (or review the diff yourself if the skill is not
available): remove dead code, extra abstractions and stray debug output the
work left behind. Commit the result.

## 4. See what CI will run, and run it locally

[choruz-pre-push-checks](../choruz-pre-push-checks/SKILL.md) owns the selection rules; the short version:

```bash
bash .agents/skills/choruz-pr/pr-plan.sh
```

It uses the same selectors as the workflow's `Detect changes` job and prints
two lists: the CI jobs this change triggers, and the commands to run locally
first. Run those commands. A red job on the PR that you could have caught
locally costs a CI round trip and a reviewer's trust.

Local e2e needs the host stack (`infra/host/web_e2e.sh` starts PostgreSQL,
the API, the pipeline and the web app itself); see `docs/testing/pr-test-policy.md`.

## 5. Open the PR

- Fill `.github/PULL_REQUEST_TEMPLATE.md` honestly: the type from step 1, the
  tests added (or why none), what you ran in step 4, the risk. If an AI agent
  wrote part of it, say so.
- Labels are how the type becomes a gate:
  - type `api`/`database` → add label `database`
  - type `security`/`auth` → add label `security`
  - want the full e2e suite on any PR → add label `ci-full`

  ```bash
  gh pr create --fill --label database          # or --label security / ci-full
  gh pr edit <number> --add-label ci-full        # later
  ```

  Without the label the full suite does not run for that type; the label is
  the author's responsibility, so add it in the same step as opening the PR.
- Commit messages: Conventional Commits, and no model names or session URLs in
  titles or bodies beyond the trailers this repository already uses.

## 6. Merge only on green, then clean up the branch

- The required check is **`CI (linux) required`**. Wait for it; do not merge
  on partial results and do not ask for a bypass.
- Code-review bots are advisory. They neither block a green required CI nor
  require a separate wait before merge unless a repository rule changes the
  required check itself.
- Red job: read the log, fix the cause, push. Never skip, disable or
  quarantine a test to get green, never push an empty commit or close/reopen
  to re-run. A failure that reproduces identically on `main` is the one case to
  stand down: say so on the PR.
- Squash-merge when green. If the branch was stacked on another PR, follow
  [choruz-merging-stacked-prs](../choruz-merging-stacked-prs/SKILL.md): retarget to
  `main` after that PR merges, merge `main` in, and let CI run once more.

After a PR reaches a terminal state, remove its source branch instead of
leaving stale remote refs behind:

- Merge only after the required check is green. Cancel a PR only when its work
  is intentionally abandoned; a red check is never a reason to delete a
  branch.
- Before deleting `HEAD_BRANCH`, verify that the PR is merged or intentionally
  closed and that no open PR uses the branch as its base:

  ```bash
  gh pr view <number> --json state,mergedAt,headRefName
  gh pr list --state open --base HEAD_BRANCH --json number,headRefName
  ```

  Retarget or finish every dependent PR first. For a stack, follow
  [choruz-merging-stacked-prs](../choruz-merging-stacked-prs/SKILL.md) rather
  than deleting an intermediate base.
- Delete the remote source branch only after those checks pass:

  ```bash
  git push origin --delete HEAD_BRANCH
  ```

  Delete the local branch too when it is not checked out by another worktree.
  Never remove the active branch or a worktree merely to satisfy cleanup.

## Checklist

- [ ] Type picked from the policy table; stricter row when in doubt
- [ ] Tests the row requires are in the diff
- [ ] `/simplify` (or a manual pass) applied
- [ ] `.agents/skills/choruz-pr/pr-plan.sh` run, and every listed local command passed
- [ ] Template filled; `database` / `security` / `ci-full` label added when the type calls for it
- [ ] For `feature`, `api` / `database`, `security` / `auth`: "Seams touched" filled from `docs/adding-a-feature.md`, every unchecked seam with its reason
- [ ] `CI (linux) required` green before merge
- [ ] After merge or intentional cancellation, PR state and dependent PRs verified; unused source branch removed from `origin` and locally
