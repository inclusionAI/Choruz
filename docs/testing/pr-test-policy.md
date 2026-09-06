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

## Behaviour acceptance evidence

Before implementing a behaviour change, identify the user's starting state, action, observable result and the boundary that owns the result. Select evidence for that contract, not just the edited files. Pure refactors retain the affected contract; prose-only changes need no invented product scenario. Changes to behavioural instructions, including skills, need a scoped example showing how the instructions guide the intended decision.

### Assertions that can reject a broken implementation

A test must distinguish the intended result from a plausible failure. A boolean type check does not prove visibility; a nonempty colour string does not prove a theme; an HTTP success or an Agent's claim does not prove a file was saved. Such assertions may support diagnostics but cannot be the sole acceptance evidence. Fixtures must make the missing behaviour observable: a search-navigation test starts with the target outside the visible history, not as the only message on screen.

For a bugfix or a new guard, run the focused regression against the unfixed behaviour or a representative negative control, observe the intended assertion fail, then restore the implementation and observe it pass. A setup error, missing dependency or unrelated failure is not the required red result. Use disposable fixtures or an isolated checkout; never mutate a live service or overwrite another contributor's changes to create a negative control. If reproducing the red state is unsafe or unavailable, record why, the substitute evidence and the unverified contract instead of claiming red/green proof.

### The entry path and the world it changes

Keep focused unit tests, but a change crossing UI, HTTP, transport, process or storage boundaries also needs an assembled test through the affected production entry path. Calling the internal provisioning function does not cover its HTTP validation; a source-only runner does not cover a packaged binary. Mock only the expensive or nondeterministic boundary needed by the test, and state what that replacement cannot prove. Do not replace the route, device dispatch, binding or persistence whose correctness the scenario claims to establish.

Verify the result at its owner: re-read the target file, message or binding; observe the terminal output or process outcome. Check important absence guarantees too, such as no duplicate delivery or no write on the controller. A UI-only scenario may use an isolated component fixture when no server behaviour is part of its claim.

### Select the relevant ordinary scenarios

Apply the rows the change affects. This is not a Cartesian product of every device, driver, theme and failure mode, and it does not require live credentials for unrelated changes.

| Affected behaviour | Required distinguishing scenario |
| --- | --- |
| Device selection, remote dispatch or device-local paths | Controller A and target B have distinct home/workspace roots and the relevant differing capability or account state. Exercise the shipped dispatch and prove the result belongs to B, not A. Separate processes on one CI host are sufficient when they preserve that boundary; a fake B handler that merely returns success is not. |
| Account selection, session discovery or resume | Default and isolated accounts have distinct profile stores and owned sessions. Verify the selected identity/profile is used and every in-scope store is considered; do not infer success from the account label alone. |
| Drafts, cached views, terminal state or navigation | Exercise the affected normal transition, such as switching away and back, reopening or resizing. Assert the promised retained state and visible layout, not only the initial mount. |
| Multi-item send/import or retry | Include partial success followed by failure and retry. Verify completed items are not duplicated, unsent work is retained and failure is not represented as an empty success. |
| Terminal/editor rendering or theme | Use stable representative output, including relevant ANSI sequences, and assert the affected content-area geometry, foreground/background or visibility in supported affected themes. Add screenshot/recorded-output evidence when DOM assertions cannot capture the defect; inspect expected-output changes rather than blindly updating them. |
| Harness protocol, login or actual model execution | Keep deterministic protocol fixtures and use the relevant [real-Harness smoke](real-harness-platform-smoke.md) or focused live check in an authorised test environment. A static smoke check or fake CLI run is not a live PASS. Missing credentials or devices are reported as blocked evidence, not silently substituted or counted as passing. |

### Evidence at handoff

The PR's Tests section links each affected contract to its owning scenario and assertion. Ran locally records the command, tested revision or working-tree scope, observed result and negative-control result where required. Risk names substitutions, blocked evidence and affected paths not exercised. Keep acceptance pending and do not declare the PR ready while required evidence is missing, unless the user explicitly accepts the limitation or narrows the scope. Even then, the untested path is not verified and the required CI check still applies. This policy does not grant access to accounts, spend or deployment authority.

The author checks this evidence before declaring the PR ready; the review procedure checks it when review is performed. These semantic requirements are not automatically enforced by the CI aggregator. A green required check is necessary for merge, but cannot justify a known broken acceptance contract. Code-review bots remain advisory; no additional bot wait or approval gate is introduced.

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

Use [choruz-pre-push-checks](../../.agents/skills/choruz-pre-push-checks/SKILL.md) to select commands for the complete outgoing scope. The CI selectors choose existing jobs and tests; they do not establish that those tests cover the acceptance contract. Add the owning scenario when it is missing, rather than rerunning a larger suite of insufficient assertions.

Human and AI authors follow the same evidence requirements. Report only checks actually executed, with blocked and untested paths distinguished from passing ones.
