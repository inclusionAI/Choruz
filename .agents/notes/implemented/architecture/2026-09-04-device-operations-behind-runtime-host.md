# Agent Note: Device operations behind one `RuntimeHost`

Status: implemented

## Problem

Every feature that touches the machine an agent runs on was written twice: the gateway spawned PTYs, provisioned Codex homes, attributed session files, browsed directories and scanned session catalogs for its own device, and `choruz-connector` carried its own copies of the directory listing and session scan for a paired device. The handlers branched on `runtime_host_id` at each call site, so a capability existed for one side, the other side, or neither, and a remote agent behaved differently from a local one for no reason except which copy had been written.

## Decision

`crates/choruz-host-runtime` is the one implementation of what Choruz does on the device where a Harness runs. `HostRequest` is the request/response vocabulary (`filesystem_home`, `filesystem_list`, `scan_sessions`, `latest_session`, `codex_prepare_home`, `codex_new_session`, `codex_anchor_matches`, `codex_import_session`) and `execute` runs one request on the current device; `terminal.rs` owns the PTY pool, the `TerminalSpec` a device needs to spawn a Harness, the per-driver CLI arguments and the process-tree container.

The gateway talks to a device only through `RuntimeHost` (`services/choruz-api-gateway/src/host_runtime.rs`): `call` for a `HostRequest`, and `ensure_terminal`, `attach_terminal`, `write_terminal`, `resize_terminal`, `close_terminal` for terminals. `LocalHost` serves the gateway's own device in-process and `RuntimeHost::Linked` sends the same calls over a remote device's host link ([one host link carries every request to a remote device](2026-09-04-host-link-for-remote-devices.md)). `RuntimeHost::for_binding` is the single place that decides which device a binding runs on. Handlers keep every database decision (binding anchors, capture windows, session identity checks) and hand the device only I/O: the split in `handlers_terminals.rs` is "what the row says" versus "what the disk says", and `RuntimeStore::session_sync_target` / `record_discovered_session_id` expose that split for the native session backfill.

The connector answers the same `HostRequest` values over its host link, so its directory listing and session scan are the crate's.

Agent creation and session import leave automatic executable selection unset. `TerminalSpec.binary_path` carries only a binding's explicit executable; `ensure_terminal` resolves the device's Harness configuration immediately before spawning. It trims configuration values and selects `CHORUZ_<HARNESS>_BINARY`, then the supported `*_CLI_PATH` alias, then the command name; MathCode has no alias. The gateway never resolves its own environment for another device. An explicit missing path fails instead of falling back to a different executable. Existing absolute binding paths remain explicit because their origin cannot be inferred safely.

`CodexPrepareHome` carries the binding's account metadata, as `TerminalSpec` does. `execute` resolves the profile with `harness_account_env` on the executing device before preparing the managed home. Account identity belongs to the controller's binding; its filesystem location belongs to the device. Sending a controller-resolved absolute account path violates that split when the two machines have different home directories.

Session scans carry active, company-owned isolated account identities to the selected device, which resolves their directories with the same account helper. Import rescans those profiles, validates the selected account, and preserves its identity in both the runtime binding and import deduplication key. `CodexImportSession` reads history from that profile rather than the device's default login. Discovered model labels are display metadata, not a new CLI override: an imported session resumes its native model.

## Alternatives considered

**Keep a `runtime_host_id` branch per handler and port each feature to the connector as it is needed.** Rejected: that is the situation this note replaces. Each port re-implements device logic, and the branches multiply with every feature.

**Move the whole terminal handler, including database writes, into the shared crate and give the connector a database connection.** Rejected: the controlled computer must not hold the controller's database credentials, and the connector must keep working through the encrypted relay without a database.

**Make the device interface a trait with per-driver methods (`spawn_codex`, `spawn_claude`).** Rejected: a driver difference is data in `TerminalSpec` and `terminal_cli_args`, not an interface difference; one spec keeps the connector unaware of drivers.

## Consequences

A device feature is written once and exists for every device the gateway can reach. The gateway's own device is a `RuntimeHost` like any other, so the remote implementation is a transport (the host link), not a second feature set. Filesystem browse roots (`CHORUZ_FS_BROWSE_ROOTS` or `HOME`) apply to the gateway's device and the connector's device by the same code.

## Testing

The `remote terminals resolve the target executable and preserve explicit target paths` regression provisions and imports through the Web/API with a real connector. It verifies target-environment and explicit-path markers, rejects a missing explicit path, and checks automatic bindings do not persist a controller executable. Only the CLI bodies are deterministic substitutes.

The `imports and resumes default and isolated account sessions on the selected device` regression scans real Claude JSONL and Codex SQLite/history stores through a connector, imports identical session IDs from different profiles, retries the import, and verifies the persisted account, copied history and terminal arguments. Its substituted CLIs check account-root markers; provider authentication is outside this test.

The `remote Codex terminal uses the selected account on its own device` regression in `apps/web/tests/e2e/terminal.spec.ts` starts a real connector with a distinct home and account root. It calls the terminal ensure endpoint for default and isolated profiles, verifies the managed authentication link targets that device's profile, and checks the CLI reads the selected marker. The CLI is a deterministic substitute; this test does not verify provider authentication.

`crates/choruz-host-runtime` carries the PTY round trip (`fake_cli_pty_round_trips_input_output_and_exit_code`), the driver argument tables and the managed Codex home and session attribution tests. `services/choruz-api-gateway/src/handlers_terminals.rs` keeps the anchor validation tests and runs the Codex import and anchor checks through `LocalHost`; the Postgres-backed `codex_terminal_open_reconciles_capture_metadata_after_gateway_restart_window` and `native_session_import_runs_end_to_end_and_is_idempotent` exercise the same path end to end. The connector test `remote_filesystem_listing_stays_inside_the_configured_root` pins the shared browse-root rule.
