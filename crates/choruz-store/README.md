# choruz-store

Event store of the durable message pipeline: `EventStore` wraps the PostgreSQL pool and owns the `conversation_events` append-only log and the `event_outbox` rows written in the same transaction. `CdcPoller` claims unpublished outbox rows (woken by LISTEN/NOTIFY, with a fallback poll) and dispatches them to an in-memory channel. `crates/choruz-application`, the router and writer crates, both Rust services and `apps/choruz-replay` depend on it.

## Entry points

- `src/pool.rs` — `EventStore`
- `src/conversation_events.rs` — `ConversationEvent`, `ConversationEventRow`, `ThreadFlags`
- `src/event_outbox.rs` — `OutboxEntry`, `OutboxRow`
- `src/cdc_poller.rs` — `CdcPoller`, `CdcPollerConfig`, `CdcPollerHandle`

## Tests

`cargo test -p choruz-store` runs serde and configuration tests without a database; the LISTEN/NOTIFY test in `src/cdc_poller.rs` runs only when `CHORUZ_LISTENER_TEST_DATABASE_URL` is set.

## Related

- [docs/subsystems/store.md](../../docs/subsystems/store.md) — `conversation_events`, `server_seq` and idempotency
- [docs/subsystems/message-pipeline.md](../../docs/subsystems/message-pipeline.md) — the CDC poller as the pipeline's intake
- [docs/architecture.md](../../docs/architecture.md)
