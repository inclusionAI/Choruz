# choruz-ids

Type-safe UUIDv7 identities of the message pipeline: one newtype per concept (`ClientMsgId`, `MessageId`, `RouteId`, `CommandId`, `TurnId`, `AttemptId`, `ToolCallId`, `DeliveryId`, `ReplyEventId`) with uniform parsing, display and serde. The pipeline stage crates, `services/choruz-api-gateway`, `services/choruz-pipeline` and `apps/choruz-replay` depend on it.

## Entry points

- `src/lib.rs` — the `define_id!` macro and the nine id types

## Tests

`cargo test -p choruz-ids`; no PostgreSQL.

## Related

- [docs/subsystems/message-pipeline.md](../../docs/subsystems/message-pipeline.md) — how the ids chain from message to reply
- [docs/architecture.md](../../docs/architecture.md)
