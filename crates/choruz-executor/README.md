# choruz-executor

Local SQLite turn records and incomplete-turn inspection. The pipeline reads these records during startup recovery; device execution belongs to `choruz-host-runtime` and uses the binding's workspace.

## Entry points

- `src/wal.rs` — `AdapterWal`, `find_incomplete_turns`
- `src/error.rs` — `ExecutorError`

## Tests

This package does not depend on the platform session store or PostgreSQL.

`cargo test -p choruz-executor`; the WAL tests use temporary SQLite files, no PostgreSQL.

## Related

- [docs/subsystems/message-pipeline.md](../../docs/subsystems/message-pipeline.md) — the headless executor loop in `services/choruz-pipeline`
- [docs/subsystems/agent-runtime.md](../../docs/subsystems/agent-runtime.md) — execution ownership and WAL
- [docs/architecture.md](../../docs/architecture.md)
