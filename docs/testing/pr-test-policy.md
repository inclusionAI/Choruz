# Pull request test policy

What a pull request has to add, and what CI runs before it can merge. The
short version: **CI decides what to run from the files you changed; the
type you declare in the PR tells reviewers what tests to expect, and can only
add gates, never remove them.**

## The two layers

1. **Declaration** (`.github/PULL_REQUEST_TEMPLATE.md`): the author picks a
   type, says which tests were added (or why none), lists what they ran
   locally and names the risk. Reviewers hold the PR to the row below.
2. **Enforcement** (`.github/workflows/ci.yml`): the `Detect changes` job
   looks at the changed paths and lists the jobs that must succeed; the
   `CI (linux) required` job fails unless every one of them did. Branch
   protection requires that job. Declaring `docs` on a PR that touches
   `migrations/` still runs the migration smoke, because CI never reads the
   declared type.

## By PR type

| Type | Tests you must add | CI runs before merge |
|---|---|---|
| `feature` | Unit/integration tests for the new behaviour; an e2e spec for a new user-facing flow | The jobs for the touched paths (below) |
| `bugfix` | A regression test that fails without the fix | The jobs for the touched paths |
| `refactor` | Usually none; existing tests of the touched module must still pass | The jobs for the touched paths |
| `ui` | Component or unit test when behaviour changes; e2e for a critical path | Web unit + typecheck + build, the e2e specs mapped to the touched feature |
| `api` / `database` | Tests for the behaviour change; a migration gets a smoke run | Rust tests, migration + API smoke; label `database` also runs the full e2e suite |
| `security` / `auth` | Tests for the new rule; e2e regression for login or permission flows | Rust/web tests; label `security` also runs the full e2e suite |
| `ci` / `build` / `deps` | Policy tests for CI scripts; nothing else | Full e2e suite (any change under `.github/**`), static checks, and whatever the touched paths select |
| `docs` / `chore` | None | Nothing: only `Detect changes` and the aggregator, about 30 seconds |

## What CI runs for which paths

| Changed path | Jobs |
|---|---|
| `crates/**`, `services/**`, `apps/choruz-*/**` | Rust lint and tests for the changed crates and every crate that depends on them (`select_rust_packages.py`), DB and API smoke, e2e (P0 set) |
| `agent-templates/**` | Rust tests for `choruz-pipeline` (it embeds the fragments), the web template unit test, e2e (P0 set) |
| `migrations/**`, `Cargo.*`, `.cargo/**`, `rust-toolchain*` | Rust lint and tests for the whole workspace, DB and API smoke, e2e (P0 set) |
| `apps/web/**`, `package.json`, `pnpm-lock.yaml` | Web: `vitest related` for the changed source files (the whole suite when the harness changes), typecheck, build; e2e for the touched feature (see below) |
| `infra/host/**`, `scripts/historical-migrations.sha256` | DB and API smoke, e2e (P0 set) |
| `services/choruz-bridge/**` | Bridge build |
| `services/remote-control-gateway/**` | Remote Control Gateway check + test |
| `infra/ops/**` | Ops lint |
| `scripts/**`, `infra/host/**` | Host lifecycle policy tests |
| `.github/**` | CI gate policy tests, full e2e suite |
| Any non-documentation file | Security scan (cargo-deny, trivy) |
| `main` (push) | Everything, plus the full e2e suite, performance smoke and release packaging |

### Which e2e specs a web change runs

`.github/scripts/select_e2e_specs.py` maps the changed files to Playwright
specs:

- A change confined to one feature (git graph, file explorer and editor,
  pixel world, detail panel, channel tasks, threads, servers and machines,
  theme, docs pages, provisioning modals and harness accounts, terminal,
  message list, chat input and attachments, chat header) runs that feature's
  specs plus `tests/e2e/app-smoke.spec.ts`, on 1–3 shards depending on how many
  tests they hold.
- An edited spec runs itself.
- A change to a shared file (`chat-app.tsx`, `sidebar.tsx`, `chat-input.tsx`
  is shared by several features and mapped accordingly, shared `lib/`,
  styles, config, test fixtures) or outside `apps/web` runs the P0 set:
  auth, company, agent, terminal, api-routes, messaging, outbox,
  conversation, websocket, attachment, machines. The specs that the
  change's mapped files select still run alongside the P0 set, so a feature
  edit that also touches an Agent Note or a public asset keeps its own
  coverage.
- Unit tests and Markdown select nothing.

The rules live in that script, with unit tests next to it; extend them when
you add a feature with its own spec.

## Asking for more

- Add the `ci-full` label to run the full e2e suite (four shards, about
  5 minutes). Later pushes keep running it.
- Labels `database` and `security` run the full suite as well.
- The full suite always runs after merge on `main`; a failure there is a
  bug to fix forward, not a reason to skip it on the next PR.

## Before you push

A documentation-only pull request such as this one runs no test job at all.

Run what CI will run, it is faster than a CI round trip:

```bash
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm web:test && pnpm web:check && pnpm web:build
pnpm web:e2e -- tests/e2e/<the spec for your feature>.spec.ts
```

Pull requests written with an AI agent follow the same rules: the person who
opens the PR has read every line, has run the relevant tests, and says so in
the template.
