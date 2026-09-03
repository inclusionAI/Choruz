# choruz-tools

Tool gateway and effect journal of the message pipeline: `ToolGateway` runs every tool call through `effect_journal` rows so a mutating call replays idempotently, `ToolRegistry` classifies each tool as read-only or mutating with a `MutationPolicy`, and `effect.rs` holds the journal types and CRUD. `services/choruz-pipeline` depends on it (`default_registry`, `ToolExecutor`, `ToolGateway`).

## Entry points

- `src/gateway.rs` — `ToolGateway`, `ToolExecutor`, `ToolCallRequest`, `ToolGatewayError`
- `src/registry.rs` — `ToolRegistry`, `MutationPolicy`, `default_registry`
- `src/effect.rs` — `EffectRecord` and the `effect_journal` CRUD

## Tests

`cargo test -p choruz-tools`; unit tests only, no PostgreSQL.

## Related

- [docs/subsystems/message-pipeline.md](../../docs/subsystems/message-pipeline.md) — where the gateway sits in the executor path
- [docs/architecture.md](../../docs/architecture.md)
