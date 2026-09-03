# choruz-replay

Debugging and auditing CLI that reads `conversation_events` and prints the rows in a human-readable or JSON format: filter by conversation and sequence range (`--conversation`, `--from-seq`, `--to-seq`), by `--turn-id` or `--command-id`, or list `--dead-letters --since 24h`. It connects with `CHORUZ_DATABASE_URL` and depends on `choruz-store` and `choruz-session` for the queries.

## Entry points

- `src/main.rs` — argument dispatch and the usage text
- `src/cli.rs` — the hand-rolled argument parser
- `src/query.rs` — the queries
- `src/output.rs` — human-readable and JSON output

## Tests

`cargo test -p choruz-replay`; unit tests only, no PostgreSQL.

## Related

- [docs/subsystems/message-pipeline.md](../../docs/subsystems/message-pipeline.md) — the events, turns and dead letters this tool replays
- [docs/architecture.md](../../docs/architecture.md)
