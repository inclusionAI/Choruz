# choruz-session

Session manager of the message pipeline: `PgSessionStore` owns the `session_registry`, `agent_commands` and `dead_letters` tables (command state machine, leases with epoch fencing, heartbeats, runtime-host command claims) and the in-memory executor registry; `retry.rs` computes the exponential backoff and exhaustion rules. `services/choruz-pipeline`, `services/choruz-api-gateway`, `choruz-router`, `choruz-executor` and `apps/choruz-replay` depend on it.

## Entry points

- `src/store.rs` — `PgSessionStore`
- `src/models.rs` — `Session`, `SessionStatus`, `AgentCommand`, `CommandStatus`, `DeadLetter` and the update structs
- `src/retry.rs` — `next_retry_at`, `is_exhausted`
- `src/error.rs` — `SessionError`, `SessionResult`

## Tests

`cargo test -p choruz-session`. `tests/integration.rs` runs against the database named by `CHORUZ_TEST_DATABASE_URL` (default `host=127.0.0.1 port=5432 user=$USER dbname=choruz`) with the migrations applied, so it needs a running PostgreSQL.

## Related

- [docs/subsystems/message-pipeline.md](../../docs/subsystems/message-pipeline.md) — leases, retries and dead letters in the dispatch loop
- [docs/subsystems/agent-runtime.md](../../docs/subsystems/agent-runtime.md) — command leases as seen from an agent session
- [docs/architecture.md](../../docs/architecture.md)
