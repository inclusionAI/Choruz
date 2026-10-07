# choruz-cli

Use `choruz` for local library operations or authenticated host operations without opening the dashboard. Host commands share the dashboard's permissions, audit records and validation; they never write PostgreSQL directly. `CHORUZ_SESSION_TOKEN` (or, on a loopback host only, `CHORUZ_OPERATOR_USER` / `CHORUZ_OPERATOR_PASSWORD`) supplies authentication. `usage()` in [src/main.rs](src/main.rs) owns the command list and global options.

## Local capabilities

`library trace` reads a JSON source descriptor with `harness` (`claude` or `codex`), `account_home`, `workspace_path`, `session_id`, and optional `cursor`. It calls the [native trace reader](../../crates/choruz-learning/README.md), returns its records and continuation cursor, and never selects an ambient account.

`library score` takes an [OutputCheck](../../crates/choruz-evaluation/src/evaluation.rs) JSON file and an answer text file. A judge check returns `score: null` and `requires_judge: true`; this command does not call an LLM. `library community` validates a JSON array of [behavior records](../../crates/choruz-community/README.md) and returns deduplicated counts. Input files are limited to 4 MiB. These commands need neither a host nor a database.

`tools status` reports local browser and desktop diagnostics. Enable/disable operations use [device tool setup](../../crates/choruz-computer-use/README.md) and persist the selection in the current user's home. Enabling waits for installation; missing browser or OS permissions remain visible as `needs_attention`. It does not grant those permissions.

## Host capabilities

`choruz start local` starts the shared API without the group-chat pipeline or a pairing request. It retains the API's asynchronous learning worker and needs PostgreSQL. `choruz start` starts the full headless stack and requests a Remote Control credential. Both reuse an existing ready host without changing its service selection. Stop that host before choosing a different composition. `--api-url` selects a loopback port for a new host. See [host composition](../../docs/subsystems/host-and-remote.md) for database and bundle requirements.

`learning` reads or changes the selected binding's existing experience policy, evaluations and community permissions. Mutation bodies are JSON files using the [HTTP contract](../../openapi/choruz.yaml). `api` exposes other `/v1/` endpoints on the configured host with the same authentication; it accepts an optional JSON body for non-GET requests. Failed HTTP responses exit nonzero. Neither command starts a host automatically or duplicates its learning worker.

## Entry points

- [src/main.rs](src/main.rs) — argument parsing, host lifecycle and control-plane commands
- [src/capabilities.rs](src/capabilities.rs) — library and authenticated API adapters

`learning prepare <binding-id> <body.json>` takes `{"input":"Your next task"}` and returns `prompt` and `revision_id`. Pass the returned prompt to your own Claude Code or Codex invocation. Preparation reads the current owned policy and runs any reviewed collaborators on the binding's device/account; it can consume that account's quota. It neither submits the foreground turn nor edits project instructions. Call it for each turn: disabling learning or clearing the revision returns the original input, and a selection change during preparation returns an error. Model calls are not retried automatically. This explicit adapter is not an automatic hook into every native CLI session; the consumer remains responsible for using the matching account, workspace and session.

## Tests

`cargo test -p choruz-cli` runs parsing and compiled-binary tests without PostgreSQL. The learning scenario in [terminal.spec.ts](../web/tests/e2e/terminal.spec.ts) also runs the binary against the host stack and reads the saved policy through the API used by the dashboard.

## Related

- [docs/subsystems/host-and-remote.md](../../docs/subsystems/host-and-remote.md) — `choruz start`, the headless host and Remote Control pairing
- [docs/architecture.md](../../docs/architecture.md)
