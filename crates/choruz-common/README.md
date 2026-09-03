# choruz-common

Types most Rust packages share: `AppError` / `AppResult`, `PgConfig` (the one reader of `CHORUZ_DATABASE_URL` and the `CHORUZ_PG_*` variables), `HostServiceStatus` and `HOST_SERVICE_PROTOCOL_VERSION` for readiness probes, the process-wide Prometheus registry, the built-in plugin ids, and the `now()` and `new_id()` (UUIDv7) helpers. Most workspace packages, including `services/choruz-api-gateway` and `services/choruz-pipeline`, depend on it.

## Entry points

- `src/lib.rs` — `AppError`, `PgConfig`, `HostServiceStatus`, `now`, `new_id`
- `src/metrics.rs` — `register_counter`, `register_counter_vec`, `register_gauge`, `register_histogram` and `text()` over the single registry each binary serves at `/metrics`
- `src/plugins.rs` — `BUILTIN_PLUGIN_IDS` and the per-plugin id constants

## Tests

`cargo test -p choruz-common`; no PostgreSQL.

## Related

- [docs/subsystems/store.md](../../docs/subsystems/store.md) — the configuration and error contract the store layer builds on
- [docs/architecture.md](../../docs/architecture.md)
