# choruz-connector

Runtime-host connector that executes Agent work on another machine. `pair` retains direct API pairing for private networks. `pair-relay` receives onboarding secrets through stdin, uses Choruz Remote Control's end-to-end-encrypted transport to register the machine without an inbound port, and stores a mode-`0600` config. `run` restores either transport, heartbeats, claims `/v1/runtime-hosts/{id}/commands/claim`, stages the message's attachments into the workspace inbox, runs each command with the headless driver from `choruz-agent-runtime`, forwards the `$CHORUZ_SEND` commands the Agent wrote to `CHORUZ_CONNECTOR_OUTBOX`, completes it with the `send` frames and ships every other command to the binding's outbox mirror on the controller (`/v1/runtime-hosts/{id}/bindings/{binding_id}/outbox`). It also keeps the host link open (`/v1/ws/runtime-hosts/link`), over which the controller runs terminals, filesystem browsing, session scans, account probes and workspace provisioning on this machine, and ships the outbox of every live terminal every two seconds.

## Entry points

- `src/main.rs` — the whole binary: direct and relay pairing, persistent transport, command execution, the host link (`maintain_host_link`), outbox shipping (`ship_outbox`, `ship_pending_outboxes`), `append_outbox_command`, and Harness login flows
- [`crates/choruz-relay-client`](../../crates/choruz-relay-client/src/lib.rs) — Rust implementation of the browser Remote Control pairing and encrypted HTTP wire contract

## Tests

`cargo test -p choruz-connector -p choruz-relay-client`; unit tests cover private config storage, protocol routing, encryption, and a real directory listing without PostgreSQL. Gateway integration tests cover the host link, remote import boundary and outbox shipping.

## Related

- [docs/subsystems/host-and-remote.md](../../docs/subsystems/host-and-remote.md) — the routes a connector calls and how a host is paired
- [docs/subsystems/agent-runtime.md](../../docs/subsystems/agent-runtime.md) — the driver and binding model the connector executes
- [docs/architecture.md](../../docs/architecture.md)
