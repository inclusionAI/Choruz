# choruz-pipeline

The single process that turns persisted conversation events into agent turns and agent output back into events: it wires `choruz-store`'s CDC poller, the router, the session store, a headless CLI executor and the writer into cooperating tokio tasks, drains every binding's Maildir outbox, refreshes workspace instruction files, and runs the cron scheduler, lease monitor and retry scheduler. Configuration comes from the environment (`CHORUZ_DATABASE_URL` or `CHORUZ_PG_*`, `CHORUZ_PIPELINE_METRICS_PORT`, `RUST_LOG`; `src/config.rs` lists the rest).

## Entry points

- `src/main.rs`, `src/pipeline.rs` — process start-up and the task topology
- `src/dispatch.rs`, `src/executor.rs` — lease pending commands and run one headless CLI turn each
- `src/outbox_watcher.rs`, `src/outbox_handler.rs` — drain `$CHORUZ_SEND` commands from `<workspace>/.choruz-outbox/new/`
- `src/instructions.rs` — `CLAUDE.md` / `AGENTS.md` bootstrap and refresh
- `src/pg_member_provider.rs`, `src/pg_result_store.rs` — the PostgreSQL implementations of the stage-crate traits

## Tests

`cargo test -p choruz-pipeline` requires a running PostgreSQL. Source `infra/host/setup_test_database.sh` first to supply `CHORUZ_TEST_DATABASE_URL` for the outbox, cron, watcher and member-provider tests; these tests fail when it is missing. `src/executor/tests.rs` creates a temporary database per test from `CHORUZ_PG_HOST`, `CHORUZ_PG_PORT`, `CHORUZ_PG_USER` and `CHORUZ_PG_PASSWORD`.

## Related

- [docs/subsystems/message-pipeline.md](../../docs/subsystems/message-pipeline.md) — one message end to end, leases, retries, ports
- [docs/subsystems/agent-protocol.md](../../docs/subsystems/agent-protocol.md) — the envelope, `$CHORUZ_SEND` and instruction bootstrap
- [docs/architecture.md](../../docs/architecture.md)
