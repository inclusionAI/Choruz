# Contributing

## Setup

Prerequisites: Rust (pinned by `rust-toolchain.toml`), Node 24 (`.nvmrc`),
pnpm 10 (`packageManager` in `package.json`) and PostgreSQL 16.

```bash
pnpm install
pnpm dev:all        # Postgres + choruz-api-gateway + pipeline
pnpm dev:web        # Next.js on http://127.0.0.1:3100
```

See the [environment-variable reference](apps/web/app/docs/operations/env-vars/page.tsx)
for runtime configuration and [AGENTS.md](AGENTS.md) for the repository layout.

## Before you open a pull request

1. Pick the change type from [docs/testing/pr-test-policy.md](docs/testing/pr-test-policy.md)
   and add the tests that type requires.
2. Run what CI will run for your paths:

   ```bash
   cargo clippy -p <crate> --all-targets -- -D warnings
   cargo test -p <crate>
   pnpm web:check && pnpm web:test          # apps/web
   pnpm web:e2e tests/e2e/<feature>.spec.ts # when the policy asks for e2e
   pnpm preflight:quick                     # everything, before a large change
   ```

3. Fill in the pull request template. Add the `database` or `security`
   label when the policy asks for it, or `ci-full` to run the whole e2e suite.

CI runs only the crates, unit tests and e2e specs your diff touches; a
documentation-only pull request runs no test job. A pull request merges when
`CI (linux) required` is green.

## Conventions

- Commit messages follow Conventional Commits (`feat(web): …`, `fix(pipeline): …`, `chore: …`).
- Rust: `cargo fmt`, clippy clean with `-D warnings`.
- Web: TypeScript strict, `pnpm web:check` must pass.
- Database changes are new files under `migrations/` (`V0NN__name.sql`);
  never edit an applied migration.
- Every non-trivial change carries an Agent Note under `.agents/notes/` (see [.agents/notes/README.md](.agents/notes/README.md)); `AGENTS.md` holds the standing rules.
