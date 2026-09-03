# AGENTS.md

Engineering rules for every agent and person working in this repository. `CLAUDE.md` is the Choruz platform protocol an agent follows to talk to humans and other agents; this file is how the code gets changed.

## Pre-release stance: foundation over blast radius

Choruz has no external users yet. Fix the root cause, delete the old path, and update every caller in the same change; do not add compatibility shims, feature flags for the transition, or "legacy" branches. A migration file is the one exception: applied migrations are frozen by checksum and get a successor, never an edit.

## Repository layout

```
apps/            human entry points: choruz-cli, choruz-replay, web
services/        long-running processes: choruz-api-gateway, choruz-pipeline, choruz-server, choruz-connector, choruz-bridge, remote-control-gateway
crates/          libraries, every package choruz-<role>: domain, application, infrastructure, auth, common, supervisor and the runtime crates
assets/          brand artwork and the provenance records for shipped visual assets
migrations/      PostgreSQL schema; 0001..0032 frozen by checksum, new files V0NN__name.sql
openapi/         the HTTP wire contract; a gateway test keeps it equal to the route table
agent-templates/ the instruction fragments every agent receives; rendered by the pipeline and by web provisioning
infra/host       the local stack: start, dev, migrate, e2e, smoke, chaos, perf and their tests; infra/ops release and backup
scripts/         repository tooling and its tests (scripts/tests)
docs/            architecture, operations, testing, data model
.agents/         Agent Notes (notes/) and skills (skills/; .claude/skills links here)
.github/         CI workflow, actions, selector scripts and their tests
```

## Commands

```bash
pnpm install                     # Node 24, pnpm 10 (packageManager), PostgreSQL 16
pnpm dev:all                     # Postgres + choruz-api-gateway + pipeline
pnpm dev:web                     # Next.js on http://127.0.0.1:3100
cargo clippy -p <crate> --all-targets -- -D warnings
cargo test -p <crate>
pnpm web:check && pnpm web:test  # apps/web typecheck and vitest
pnpm web:e2e tests/e2e/<feature>.spec.ts
pnpm preflight:quick             # the local approximation of CI
python3 scripts/verify_agent_notes.py
python3 -m unittest discover scripts/tests
```

### Run relevant checks locally

CI runs what the changed paths need, and a pull request merges only on a green `CI (linux) required`. Select evidence with [choruz-pre-push-checks](.agents/skills/choruz-pre-push-checks/SKILL.md): its helper `.agents/skills/choruz-pr/pr-plan.sh` uses the workflow's own selectors to print the jobs the change triggers and the commands to run first. Match evidence to the surface (a focused test for a behaviour change, the e2e spec for a user-visible flow, the notes verifier for a note) and do not default to the full suite; `ci-full` is a label, not a habit. Report only commands actually run.

## Conventions

- **Every non-trivial change includes an Agent Note in the same pull request**; only mechanical or local edits are exempt ([scope](.agents/notes/README.md#when-to-write-one)). Archived notes are frozen: never edit them or treat them as current authority ([archive policy](.agents/notes/README.md#archiving-and-deletion)).
- **Pick the change type first.** [docs/testing/pr-test-policy.md](docs/testing/pr-test-policy.md) maps each type to the tests it must add; the stricter row wins when a diff spans two. A `feature`, `api`, `database`, `security` or `auth` change walks [docs/adding-a-feature.md](docs/adding-a-feature.md) and records the seams it touched in the pull request. The pull request template asks for the type, the tests, what ran locally and the risk; fill it honestly and add the `database` or `security` label when the type calls for it.
- **Tests describe behaviour, not correctness.** Change obsolete behaviour together with its tests and say why in the pull request. A test that passes only when it runs alone is a defect in the test: own every resource it acquires (see [e2e conventions](docs/testing/pr-test-policy.md)).
- **Never skip, disable or quarantine a test to get green**, never push an empty commit or close and reopen a pull request to re-run CI. A red job is read, fixed and re-pushed.
- **Migrations are append-only.** `scripts/historical-migrations.sha256` pins the applied files; a change to schema or data is a new `V0NN__name.sql`.
- **Contracts first.** A wire, database or configuration format change updates `openapi/` or the migration before the code that uses it; `contracts::openapi_documents_every_route` in the gateway tests fails on a route the spec does not list.
- **Workspace scoping is mandatory.** Every command in `crates/choruz-application` checks `workspace_id`; a new table carrying user data carries it too.
- **Names state a role and follow one rule per language**: Rust packages `choruz-<role>` in `crates/` (libraries), `services/` (long-running processes) or `apps/` (human entry points); npm packages `@choruz/<role>`; see [naming](.agents/skills/choruz-pr/SKILL.md#naming-new-packages-and-files).
- **Repository language is English.** Prose, comments, tests and user-visible text do not add partial translations or Han-script text; see the [language decision](.agents/notes/implemented/process/2026-09-03-english-only-repository.md).
- **Rust:** `cargo fmt`, clippy clean under `-D warnings`, errors carry context, no `unwrap` outside tests and startup.
- **Web:** TypeScript strict, `pnpm web:check` clean, one `Modal` shell and the shared form fields for dialogs, no product copy hard-coded where a shared component exists.
- **Comments state non-obvious contracts**, not what the code already says and not change history. Reference documents by path, never by number.
- **Commits** follow Conventional Commits (`feat(web): …`, `fix(pipeline): …`, `chore: …`). No model names or session identifiers in commit titles or bodies beyond the trailers this repository already uses.
- Files end with exactly one trailing newline.

## Skills

Repeatable work has a skill under `.agents/skills/`, one `SKILL.md` each: [choruz-pr](.agents/skills/choruz-pr/SKILL.md) (open and merge a pull request), [choruz-pre-push-checks](.agents/skills/choruz-pre-push-checks/SKILL.md), [choruz-ci-test-reliability](.agents/skills/choruz-ci-test-reliability/SKILL.md), [choruz-code-review](.agents/skills/choruz-code-review/SKILL.md), [choruz-merging-stacked-prs](.agents/skills/choruz-merging-stacked-prs/SKILL.md), [choruz-doc](.agents/skills/choruz-doc/SKILL.md), [choruz-prose-standard](.agents/skills/choruz-prose-standard/SKILL.md), [choruz-find-simplifications](.agents/skills/choruz-find-simplifications/SKILL.md), [choruz-archive-agent-notes](.agents/skills/choruz-archive-agent-notes/SKILL.md). A skill is an operating procedure, not a product spec; it links its sources of truth instead of restating them.

## Documentation

`docs/architecture.md` is the map of how the system works today; `docs/operations/` is how to run it; `docs/testing/` is what CI runs and why; `.agents/notes/` is why things are the way they are. [docs/adding-a-feature.md](docs/adding-a-feature.md) is the checklist of seams a new feature plugs into. Docs accompany every code change: a moved file, renamed key or changed default updates the document that names it in the same pull request. Document current state, not history: no "previously", "now", "no longer".

## Editing these instructions

Keep each standing order to one to three lines that link their home; rationale lives in an Agent Note, not here. A change to this file is a `process` change and gets its own note.
