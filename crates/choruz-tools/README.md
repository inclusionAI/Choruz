# choruz-tools

Tool gateway and PostgreSQL-backed effect-journal utilities. `ToolRegistry` classifies calls as read-only or mutating with a `MutationPolicy`; `effect.rs` holds journal types and CRUD. The platform does not currently dispatch native CLI tool calls through this library. It therefore provides no platform-wide tool interception or replay guarantee.

## Entry points

- `src/gateway.rs` — `ToolGateway`, `ToolExecutor`, `ToolCallRequest`, `ToolGatewayError`
- `src/registry.rs` — `ToolRegistry`, `MutationPolicy`, `default_registry`
- `src/effect.rs` — `EffectRecord` and the `effect_journal` CRUD

## Tests

`cargo test -p choruz-tools`; unit tests only, no PostgreSQL.

## Related

- [docs/subsystems/message-pipeline.md](../../docs/subsystems/message-pipeline.md) — the separate native CLI execution path
- [docs/architecture.md](../../docs/architecture.md)
