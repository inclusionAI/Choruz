# choruz-cli

The `choruz` binary: a scriptable control-plane client that talks only to the authenticated HTTP API the web dashboard uses, so CLI and web share permissions, audit records and validation. Commands: `status`, `start` (launches the bundled headless host and prints a Remote Control pairing credential), `company list`, `agent list`, `remote status` and `remote pairing-credential`; `--api-url`, `--pipeline-url`, `--token` and `--json` are the global options, and `CHORUZ_SESSION_TOKEN` (or, on a loopback host only, `CHORUZ_OPERATOR_USER` / `CHORUZ_OPERATOR_PASSWORD`) supplies authentication.

## Entry points

- `src/main.rs` — the whole binary; `usage()` is the source of truth for the command list

## Tests

`cargo test -p choruz-cli`; unit tests in `src/main.rs`, no PostgreSQL.

## Related

- [docs/subsystems/host-and-remote.md](../../docs/subsystems/host-and-remote.md) — `choruz start`, the headless host and Remote Control pairing
- [docs/architecture.md](../../docs/architecture.md)
