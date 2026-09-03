# Agent Note: Package naming and placement

Status: implemented

## Problem

Package names followed two conventions at once: ten crates carried the `choruz-` prefix while `common`, `auth`, `domain`, `proto`, `application`, `infrastructure` and `agent-runtime` did not, so an import such as `use auth::` did not say whether the name was a workspace crate or a dependency, and a generic bare name collides with any published crate of the same name the moment two of them meet in one build graph. Directory placement had no rule either: `api-gateway` and `choruz-bridge` sat under `services/` while `choruz-pipeline`, `choruz-server`, `choruz-connector` and `remote-control-gateway`, all long-running processes, sat under `apps/`. The `choruz-host` crate was named like a process although it is the embedded-Postgres and child-process supervision library that `choruz-server` links.

## Decision

Every Rust package is `choruz-<role>`; the directory equals the package name and the crate ident is `choruz_<role>`. npm packages are `@choruz/<role>`. `crates/` holds libraries, `services/` holds long-running processes (`choruz-api-gateway`, `choruz-pipeline`, `choruz-server`, `choruz-connector`, `choruz-bridge`, `remote-control-gateway`) and `apps/` holds human entry points (`choruz-cli`, `choruz-replay`, `web`). `choruz-host` is `choruz-supervisor`, named for what it does. The rule is stated once, in the naming section of the `choruz-pr` skill, and `AGENTS.md` links it; the repository layout table lists the packages by their new homes. Binary names follow package names, so the gateway binary, its log file and the service labels in `infra/host` are `choruz-api-gateway`.

## Alternatives considered

**Bare names everywhere with `publish = false`.** rust-analyzer and Zed do this. Rejected because it would have widened the generic-name problem from seven crates to seventeen (`store`, `session`, `events`, `workspace`, `tools` are as generic as `common`), and because the binaries and the SDK already carry the `choruz-` brand.

**Prefix only the crates that might be published.** Rejected because it keeps two conventions and the judgement of "might be published" changes over time; one rule needs no judgement.

**Leave the daemons under `apps/` and define `apps/` as "anything with a `main`".** Rejected because `web` and `choruz-cli` are what a person runs and the daemons are what an operator deploys; the split by audience is the one that helps a reader find a process.

**Keep `choruz-host` as the crate name.** Rejected because the name suggested a running host process while `choruz-server` is that process; a library named for its role reads correctly from the dependency list alone.

## Consequences

`use choruz_domain::` and `use choruz_router::` read the same, and no workspace crate can collide with a crates.io name. A contributor adding a package reads one paragraph to know its name and directory. Every path and package reference in `Cargo.toml` files, `use` paths, CI selectors, host scripts, release scripts, documents and implemented Agent Notes changed in one pull request. The `pipeline` package includes the web agent templates by a relative path that now crosses from `services/` to `apps/`.

## Testing

`cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` prove every reference resolves; the CI policy tests under `.github/scripts/tests` and `scripts/tests` pin the selector fixtures with the new package names; the host e2e launcher starts `choruz-api-gateway` by its new binary name.

## Related

- [choruz-pr skill, naming section](../../../skills/choruz-pr/SKILL.md#naming-new-packages-and-files)
- [AGENTS.md repository layout](../../../../AGENTS.md#repository-layout)
- [docs/architecture.md](../../../../docs/architecture.md)
