# choruz-server

Headless Choruz host for the SSH side of a remote connection: it starts the embedded PostgreSQL and the `choruz-api-gateway` and `choruz-pipeline` children through `choruz-supervisor`, waits for their `/readyz` contracts, prints `CHORUZ_LISTENING=<gateway_port>` on stdout so the client can open its tunnel, and blocks until SIGINT or SIGTERM. It never starts the Next.js client; the local client renders the UI and proxies `/api/v1/*` over the tunnel.

## Entry points

- `src/main.rs` — the whole binary

## Tests

The binary has no tests; `cargo test -p choruz-server` only compiles it.

## Related

- [docs/subsystems/host-and-remote.md](../../docs/subsystems/host-and-remote.md) — the handshake, the tunnel and the client that starts this binary
- [docs/architecture.md](../../docs/architecture.md)
