# choruz-events

Event envelope and topic payloads of the message pipeline: `EventEnvelope` wraps every bus message, and `topics.rs` holds one payload struct per topic (`ConversationEventPayload`, `RouteDecisionPayload`, `AgentCommandPayload`, `AgentResultPayload`, `ToolEffectPayload`, `DeadLetterPayload`, `DeliveryPayload`). The stage crates `choruz-store`, `choruz-router`, `choruz-session`, `choruz-executor`, `choruz-writer` and `choruz-fanout`, and `services/choruz-pipeline`, depend on it.

## Entry points

- `src/envelope.rs` — `EventEnvelope`
- `src/topics.rs` — the payload structs

## Tests

`cargo test -p choruz-events`; no PostgreSQL.

## Related

- [docs/subsystems/message-pipeline.md](../../docs/subsystems/message-pipeline.md) — the stages that produce and consume these payloads
- [docs/architecture.md](../../docs/architecture.md)
