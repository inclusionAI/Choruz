# choruz-writer

Conversation writer of the message pipeline: `commit_result` and `run_writer_loop` take `AgentResult` values from the executor, keep only succeeded turns, and commit each as a reply row in `conversation_events` through a `ResultStore`; the `turn_id` UNIQUE constraint is the final barrier against a second reply for the same turn. `services/choruz-pipeline` depends on it and supplies the PostgreSQL `ResultStore`.

## Entry points

- `src/writer.rs` — `commit_result`, `run_writer_loop`, `ResultStore`, `WriterConfig`
- `src/models.rs` — `AgentResult`, `AgentResultStatus`, `CommandAttemptRef`, `WriteOutcome`

## Tests

`cargo test -p choruz-writer`; in-memory stores, no PostgreSQL.

## Related

- [docs/subsystems/message-pipeline.md](../../docs/subsystems/message-pipeline.md) — the writer stage and the dedup barrier
- [docs/subsystems/sync-feed.md](../../docs/subsystems/sync-feed.md) — how a committed reply reaches the browser
- [docs/architecture.md](../../docs/architecture.md)
