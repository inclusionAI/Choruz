# choruz-domain

Row types of the control plane with no I/O: `Principal`, `Company`, `CompanyMember`, `User`, `Conversation`, `ConversationMember`, `Message`, `ReadReceipt`, `AuditLog` and `EventEnvelope`, with the `PrincipalType`, `ChannelVisibility` and `ConversationType` enums. `crates/choruz-application` and `services/choruz-api-gateway` depend on it.

## Entry points

- `src/lib.rs` — the whole crate; there are no submodules

## Tests

The crate has no tests; `cargo test -p choruz-domain` only compiles it.

## Related

- [docs/subsystems/store.md](../../docs/subsystems/store.md) — the tables these rows come from
- [docs/architecture.md](../../docs/architecture.md)
