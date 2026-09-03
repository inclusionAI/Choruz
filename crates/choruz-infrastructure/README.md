# choruz-infrastructure

Tracing setup for every Choruz Rust binary: `init_tracing(service_name)` writes to stderr, reads the standard `RUST_LOG` filter and selects human-readable or JSON output through `CHORUZ_LOG_FORMAT=human|json`. The defaults are `info` and `human`; invalid values fail startup.

## Entry points

- `src/lib.rs` — `init_tracing`

## Tests

Unit tests pin the configuration defaults and reject invalid filter and format values.

## Related

- [docs/subsystems/store.md](../../docs/subsystems/store.md) — lists this crate with the other layered crates
- [docs/architecture.md](../../docs/architecture.md)
