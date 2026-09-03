# choruz-api-gateway

The Axum process that serves every `/v1` route, the terminal and dashboard-sync WebSockets and the health endpoints. `main.rs` initialises tracing, loads `Config::from_env` and serves `router_with_runtime`, which registers the routes, builds `ApiState` (`DbService`, `RuntimeStore`, `PgSessionStore`, the PTY pool) and applies the request-logging middleware; each request is authenticated from a session token or an agent secret, and every durable read and write goes through `choruz-application`.

## Entry points

- `src/main.rs` — process start-up
- `src/lib.rs` — `router_with_runtime` and the route table
- `src/config.rs`, `src/state.rs`, `src/auth.rs`, `src/local_auth.rs` — configuration, `ApiState`, request authentication
- `src/handlers_*.rs` — one file per route family (conversations, messages, threads, channel tasks, runtime, runtime hosts, SSH, remote control, the sync WebSocket)
- `src/plugins/` — the built-in host plugins (kanban, pixel world, workspace git, remote SSH, remote control, agent skills)

## Tests

`cargo test -p choruz-api-gateway`. `src/tests/` creates a temporary database per test from `CHORUZ_PG_HOST`, `CHORUZ_PG_PORT`, `CHORUZ_PG_USER` and `CHORUZ_PG_PASSWORD`, so the suite needs a running PostgreSQL.

## Related

- [docs/subsystems/api-gateway.md](../../docs/subsystems/api-gateway.md) — routes, authentication and validation
- [docs/subsystems/sync-feed.md](../../docs/subsystems/sync-feed.md) — the `/v1/bootstrap` and `/v1/ws/sync` contract
- [docs/architecture.md](../../docs/architecture.md)
