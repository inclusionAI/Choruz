# choruz-application

The service layer behind the API gateway: `DbService` runs every workspace-scoped read and write of companies, principals, conversations, messages, channel tasks, sync changes and audit rows against PostgreSQL through `choruz-store`'s `EventStore`, and `ChatApp` is the in-memory shell that shares its request and response types. `services/choruz-api-gateway` and `services/choruz-pipeline` depend on it.

## Entry points

- `src/db_service/mod.rs` — `DbService`, `RateLimiter`; one submodule per table family (`companies`, `principals`, `conversations`, `messages`, `events`, `sync`, `group_workflow_tasks`, `audit`)
- `src/lib.rs` — `ChatApp` over process-local in-memory `State`
- `src/types.rs` — the request and response structs both shells share

## Tests

`cargo test -p choruz-application`; the tests run in memory and need no PostgreSQL.

## Related

- [docs/subsystems/store.md](../../docs/subsystems/store.md) — `DbService`, workspaces, `server_seq`, idempotency
- [docs/subsystems/sync-feed.md](../../docs/subsystems/sync-feed.md) — the `sync_change` rows `db_service/sync.rs` writes
- [docs/architecture.md](../../docs/architecture.md)
