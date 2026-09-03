# choruz-connector

Runtime-host connector that executes agent turns on another machine: `pair` registers the machine with a gateway and stores the connector config, and `run` loops on heartbeat, claims `/v1/runtime-hosts/{id}/commands/claim`, runs each claimed command with the headless driver from `choruz-agent-runtime`, forwards the `$CHORUZ_SEND` commands the agent wrote to `CHORUZ_CONNECTOR_OUTBOX`, and completes the command; it also claims and completes harness-account logins (Claude, Codex) on behalf of the host.

## Entry points

- `src/main.rs` — the whole binary: `pair`, `run`, `execute`, `append_outbox_command`, the harness-login flows

## Tests

`cargo test -p choruz-connector`; unit tests in `src/main.rs`, no PostgreSQL.

## Related

- [docs/subsystems/host-and-remote.md](../../docs/subsystems/host-and-remote.md) — the routes a connector calls and how a host is paired
- [docs/subsystems/agent-runtime.md](../../docs/subsystems/agent-runtime.md) — the driver and binding model the connector executes
- [docs/architecture.md](../../docs/architecture.md)
