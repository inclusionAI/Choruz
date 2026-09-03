# choruz-supervisor

Host-side plumbing for a self-contained Choruz backend: `EmbeddedPg` downloads and runs a private PostgreSQL 16 from the user's data directory and applies `migrations/*.sql`, and `Supervisor` spawns `choruz-api-gateway` and `choruz-pipeline` as child processes, waits for their versioned `/readyz` responses and stops them with the parent. `services/choruz-server` depends on it.

## Entry points

- `src/pg.rs` — `EmbeddedPg` and the migration runner
- `src/supervisor.rs` — `Supervisor`, readiness probing, child shutdown

## Tests

`cargo test -p choruz-supervisor`; the tests cover migration splitting, readiness probing against a local HTTP stub and child reaping, no PostgreSQL.

## Related

- [docs/subsystems/host-and-remote.md](../../docs/subsystems/host-and-remote.md) — `choruz-server`, the CLI and the SSH handshake built on this crate
- [docs/architecture.md](../../docs/architecture.md)
