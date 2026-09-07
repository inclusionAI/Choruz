# Agent Note: One host link carries every request to a remote device

Status: implemented

## Problem

A paired device could only reach the controller, never the other way round: the connector dialled the API (directly or through the encrypted relay) and the controller had no way to initiate anything. Every remote capability was therefore shaped as a database queue the connector polled: `agent_commands` for turns, `harness_account_login` for sign-ins and `runtime_host_operation` for directory listings and session scans. A terminal cannot be a queue. A direct message with an agent on a remote device was downgraded to a message transcript (`interaction_mode: "message"`) while the same agent on the gateway's device showed a live terminal, and every new device feature needed its own queue, claim route and connector loop.

## Decision

For Claude and Codex, the [structured direct conversation decision](../feature/2026-09-06-structured-agent-dm.md)
extends this link with session requests and supersedes the xterm-only presentation
described below. Transport and device ownership remain shared with local execution.

A connected device holds one WebSocket to the controller, `GET /v1/ws/runtime-hosts/link`, dialled by the connector as a direct socket when the API is reachable or as a relay stream (`RelayHttpClient::open_stream`, the same `stream.*` frames a paired browser uses) when it is not. The first frame is a `hello` carrying the host token, because a relayed socket arrives with the paired browser's identity rather than the device's. From then on the controller sends `call` frames (`LinkRequest`: any `HostRequest`, or a terminal operation) and the device answers with `result` frames; terminal output streams back as `terminal_output` frames until `terminal_exit`. The vocabulary lives in `crates/choruz-host-runtime/src/link.rs`, the device side is `run_device_link` over a pair of channels so the transport is the caller's choice, and the controller side is `services/choruz-api-gateway/src/host_link.rs` (`HostLinkHub`, `HostLink::call`).

`RuntimeHost::Linked` ([device operations behind one `RuntimeHost`](2026-09-04-device-operations-behind-runtime-host.md)) is the only consumer: `for_binding` reads `config_json.runtime_host_id` and returns the linked device or `409 runtime host … is not connected`. The terminal WebSocket bridge, the session import, the Codex home preparation, the harness account probe (`POST /v1/companies/{company_id}/harness-accounts/{account_id}/probe`, `choruz_harness_login::probe_account` on the device) and the dashboard's `POST /v1/runtime-hosts/{host_id}/operations` (browsing, scans and `workspace.provision` for agent provisioning) do not know whether the device is local. The instruction bootstrap (`choruz_host_runtime::instructions`) runs on whichever device executes the turn. An imported or provisioned terminal binding on a remote device is `interaction_mode: "terminal"` like a local one; the web never writes `message` for a remote binding.

`runtime_host_operation` and its claim/complete routes are gone (`V042__drop_runtime_host_operations.sql`). Turns and sign-ins keep their queues: they are long jobs that must survive a dropped link and be retried, which is what the queue semantics buy; a directory listing or a keystroke has no value once its caller has gone.

## Alternatives considered

**Reverse the pairing so the controller becomes a relay device of the remote host.** Rejected: the remote machine would need a full gateway with its own relay bridge and ticket issuance, doubling the deployment for the sake of connection direction.

**Extend `runtime_host_operation` with a `pty.*` kind and poll it faster.** Rejected: a terminal needs a bidirectional stream with sub-second latency in both directions; a leased row with a 100 ms poll is neither, and every keystroke would be a database write.

**Give the link its own binary framing.** Rejected: the relay chunks frames at 384 KiB and re-encodes them as base64 inside AES-GCM envelopes, so binary framing gains nothing; JSON text frames with base64 terminal bytes keep one decoder on both sides and stay inspectable.

**Replace the host heartbeat with link presence.** Not done: the heartbeat route stays the presence signal so a device whose link is briefly reconnecting is not listed offline; the link's hello also records `last_seen_at`.

## Consequences

A remote agent's direct message is the same xterm surface as a local one, including Codex session capture and resume, because the same handler drives both. A device that is paired but not running its connector answers every request with 409 rather than a 70-second wait. The connector keeps its terminals in the same `TerminalPool` the gateway uses locally, so a link that drops and reconnects re-attaches to a still-running terminal; a browser socket closing still kills the terminal, as it does locally. The relay's per-frame cap means a single terminal frame over the relay is at most 384 KiB of base64; larger bursts are split by the relay, not by the link. Group turns on a remote device are unchanged: still headless, still through `agent_commands`; the commands such a turn (or a remote terminal) writes into its outbox reach the pipeline through the [outbox mirror](2026-09-04-remote-outbox-mirror.md).

## Testing

Agent creation uses `drivers.inspect` over the same link for installed executables and default-profile model discovery, and delegates custom workspace validation to the selected device. Controller-local catalogs or home-directory checks would silently reintroduce a second device owner before dispatch. `apps/web/tests/e2e/device-provisioning.spec.ts` runs the real connector with a separate home and a deterministic external CLI, then exercises UI creation and verifies the device's model, workspace protocol file and persisted binding. Existing account probes and model-output parsers are reused rather than replicated in a second discovery implementation.

`crates/choruz-host-runtime/src/link.rs` pins the hello/welcome exchange, request errors on the wire and terminal streaming over in-memory channels (`device_link_streams_an_attached_terminal_and_its_exit`). `services/choruz-api-gateway/src/tests/runtime.rs`: `runtime_host_pairing_is_single_use_and_host_token_is_revocable` drives a scripted device over a real WebSocket for `filesystem.list`, a remote scan and import (asserting `interaction_mode: "terminal"`) and the 409 once the link drops; `remote_terminal_binding_streams_through_the_host_link` runs the real `run_device_link` with a fake CLI and opens `/v1/ws/terminals/{binding_id}` as the human, asserting the bytes flow both ways, that the process runs in the device's pool, and that closing the browser socket ends it.
