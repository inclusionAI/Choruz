---
name: choruz-pre-push-checks
description: Use before pushing, marking ready for review, or claiming checks pass on a Choruz branch, to select the smallest tests and checks that cover the outgoing diff without reflexively running the full repository suite.
---

# Choruz pre-push checks

Run relevant local evidence once before a push. There are no git hooks in this repository; CI owns exhaustive coverage, and it runs only what the changed paths need. A red job that could have been caught locally costs a CI round trip and a reviewer's trust.

## Inspect the outgoing change

1. Confirm the checkout and branch.

```sh
git status --short --branch
git fetch -q origin main
```

2. Print what CI will run for this diff and what to run first. The helper uses the workflow's own selector scripts, so its answer matches the pull request's `Detect changes` job:

```sh
bash .agents/skills/choruz-pr/pr-plan.sh            # against origin/main
bash .agents/skills/choruz-pr/pr-plan.sh <base-ref>  # for a stacked branch
```

## Select relevant evidence

Every behaviour change needs the narrowest available test that would fail for its regression; add broader checks only for surfaces the diff reaches. When the change adds or changes a resource-owning, parallel, or asynchronous test, apply [choruz-ci-test-reliability](../choruz-ci-test-reliability/SKILL.md) first.

- **Rust crate or service:** `cargo fmt --check`, then `cargo clippy -p <crate> --all-targets -- -D warnings` and `cargo test -p <crate>` for the crate and its dependents (`pr-plan.sh` lists them). Integration tests that need PostgreSQL use `infra/host/setup_test_database.sh`.
- **Web source under `apps/web`:** `pnpm --dir apps/web exec vitest related --run <changed files>` (the whole suite, `pnpm web:test`, only when the harness or a shared config changed), then `pnpm web:check`. Run `pnpm web:build` when `next.config`, `app/` routing, or a dependency changed.
- **User-visible flow:** the e2e spec that owns it, through the host stack: `bash infra/host/web_e2e.sh tests/e2e/<feature>.spec.ts`. Use `--repeat-each=3` for a test you just fixed.
- **Migration or `crates/choruz-application` data path:** the DB smoke, `pnpm db:migration:smoke`, and the API smoke, `pnpm api:smoke`. An applied migration is checksum-frozen (`scripts/historical-migrations.sha256`): never edit one, add a successor.
- **Agent Note, `AGENTS.md`, skill:** `python3 scripts/verify_agent_notes.py` and `python3 -m unittest discover scripts/tests`.
- **CI workflow or selector script:** `python3 -m unittest discover .github/scripts/tests` and a YAML parse of `ci.yml`; the pull request itself runs the full suite once, by design.
- **Contracts (`openapi/`):** `cargo test -p choruz-api-gateway contracts` (the spec must list every registered route and nothing else).
- **Bridge, remote-control gateway, ops scripts:** their build or lint (`pnpm --dir services/choruz-bridge build`, the gateway checks in `ci.yml`, `pnpm ops:check`).

Do not repeat a passing check merely because a commit or push follows. Do not run `pnpm preflight:full` by default; it is for an irreducibly repository-wide change or for diagnosing CI.

## Handle failures

If a relevant check fails, stop and fix or explain the blocker. Do not push and hope CI differs. "Flake" is not a root cause: a test that passes only when run alone is a defect in the test, and [choruz-ci-test-reliability](../choruz-ci-test-reliability/SKILL.md) owns the fix. Never skip, disable, or quarantine a test to get green.

If a failure looks environment-specific, prove it: record the exact command, the failing test, and the platform-specific mismatch; confirm the non-platform evidence; prefer fixing the nondeterminism.

## Push procedure

1. Run the selected checks once.
2. Commit with a Conventional Commits message; no model names or session identifiers beyond the trailers this repository already uses.
3. Push normally. Rewriting history is allowed only on a branch you created and never on someone else's; use `--force-with-lease`, never raw `--force`.
4. Verify the remote ref matches local `HEAD`, then watch the pull request checks: `CI (linux) required` is the one that gates the merge.

Report pending checks as pending. Inspect a failure before attributing it to the branch or the environment. If no check ever starts, read `mergeStateStatus` first: GitHub creates no `pull_request` run while a PR is conflicting, and resolving the conflict is the only fix.
