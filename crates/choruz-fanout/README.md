# choruz-fanout

Fanout gateway of the message pipeline: `FanoutGateway` keeps the registry of connected clients, polls an `EventSource` for new `conversation_events` rows and pushes them to every subscriber, `CursorStore` records per-client read positions for replay on reconnect, and `ws_fanout_routes` mounts the user-scoped `GET /ws/fanout` WebSocket. `services/choruz-pipeline` depends on it and serves the routes on `CHORUZ_PIPELINE_METRICS_PORT`.

## Entry points

- `src/gateway.rs` — `FanoutGateway`, `EventSource`, the fanout loop
- `src/ws.rs` — `WsFanoutState`, `ws_fanout_routes`
- `src/cursor.rs` — `CursorStore`, `InMemoryCursorStore`
- `src/models.rs` — `FanoutEvent`, `Subscription`, `ClientCursor`

## Tests

`cargo test -p choruz-fanout`; in-memory sources only, no PostgreSQL.

## Related

- [docs/subsystems/message-pipeline.md](../../docs/subsystems/message-pipeline.md) — where the fanout loop runs
- [docs/architecture.md](../../docs/architecture.md)
