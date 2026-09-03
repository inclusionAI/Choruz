# choruz-executor

Executor-side building blocks the pipeline uses to run an agent turn: `SandboxManager` provisions the per-session workspace directory, `AdapterWal` logs prompt injections and responses to a local SQLite write-ahead log for crash recovery, and `CliAdapter` is the trait a CLI adapter (`codex_adapter.rs`) implements. `services/choruz-pipeline` depends on it.

## Entry points

- `src/sandbox.rs` — `SandboxManager`, `WorkspaceConfig`
- `src/wal.rs` — `AdapterWal`, `find_incomplete_turns`
- `src/adapter.rs` — `CliAdapter`, `CliResponse`
- `src/codex_adapter.rs` — the Codex adapter
- `src/error.rs` — `ExecutorError`

## Tests

`cargo test -p choruz-executor`; the WAL tests use temporary SQLite files, no PostgreSQL.

## Related

- [docs/subsystems/message-pipeline.md](../../docs/subsystems/message-pipeline.md) — the headless executor loop in `services/choruz-pipeline`
- [docs/subsystems/agent-runtime.md](../../docs/subsystems/agent-runtime.md) — sandbox and WAL as seen from a running agent
- [docs/architecture.md](../../docs/architecture.md)
