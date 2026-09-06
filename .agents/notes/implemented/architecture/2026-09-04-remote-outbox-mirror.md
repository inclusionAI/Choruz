# Agent Note: A remote binding's outbox is mirrored on the controller and its inbox is staged on the device

Status: implemented

## Problem

An agent on a remote device speaks to the platform by writing JSON commands through `$CHORUZ_SEND`. The local pipeline drains `<workspace>/.choruz-outbox/new` and handles every command type (`send`, `share_file`, `provision_agent`, `create_group`, `set_cron`, `task_*`). A remote device has no operator session or control-plane database, so it cannot execute those commands itself. The [host link](2026-09-04-host-link-for-remote-devices.md) made controller-initiated operations identical on local and remote devices; the agent's outbound commands needed the inverse path.

Shipping also crosses an acknowledgement boundary. If the controller persisted a command but its HTTP response was lost, the device retried the same file. Recreating that file after the pipeline had drained it executed the command twice.

## Decision

The device ships, the controller stores, and the pipeline drains. `crates/choruz-host-runtime/src/outbox.rs` owns the shared mechanics:

- `collect_outbox_commands(workspace)` reads workspace command files in helper order and attaches the bytes of a `share_file` target. The target is base64-encoded, at most `MAX_SHIPPED_FILE_BYTES` = 8 MiB, and must remain inside the workspace after symlinks are resolved. Command files remain until `CollectedOutbox::delivered` removes them. `commands_from_frames` performs the same conversion for a headless connector turn, excluding `send` because the completion already carries it. Commands from a temporary headless outbox that could not be delivered are persisted under `<CHORUZ_RUNTIME_DIR>/outbox-spool/<binding_id>` until a later pass succeeds.
- The connector posts to `/v1/runtime-hosts/{host_id}/bindings/{binding_id}/outbox` after a headless turn and every two seconds for live terminals and the spool. `shipments` limits each request to `MAX_COMMANDS_PER_SHIPMENT` = 200 commands and `MAX_SHIPMENT_BYTES` = 16 MiB of serialized commands. The gateway accepts only the binding's current host and rejects oversized file encodings before decoding them. `ShipError` separates permanent client refusals from transient delivery failures, so an invalid batch is dropped while a request that did not get through remains queued.
- `store_shipped_commands` mirrors a remote binding under `<CHORUZ_RUNTIME_DIR>/remote-outbox/<binding_id>/<host_id>`. Commands use `.choruz-outbox/new`; attached files use the workspace-relative paths named by the command. Every command name also has a SHA-256 receipt for the complete shipped command. The store writes `receipts/pending/<name>` before publishing the command and atomically moves it to `receipts/accepted/<name>` afterwards. The pipeline ignores a JSON command while its pending receipt exists. A retry with an accepted name and the same digest is a no-op even after the command file has been drained; the same name with different content is a conflict. Local and legacy command files have no pending receipt and remain immediately eligible.

The mirror is keyed by the shipping host as well as the binding. `remote_outbox_mirrors` and the pipeline's `binding_outbox_dirs` drain every mirror of a remote binding, so a shipment accepted while `assign_binding_host` moves the binding is still processed. After the move, later shipments from the former host are refused. `should_drain_binding` drains every remote binding regardless of driver because a remote headless turn ends on the device.

`provision_agent` carries the requesting agent's `runtime_host_id` through `lookup_agent_runtime_host`, so the new teammate runs beside its requester.

The inbound direction has the same placement rule. `crates/choruz-host-runtime/src/inbox.rs` stages attachments into `<workspace>/.choruz-inbox/<attachment_id>/<filename>` on the device that runs the turn. The pipeline fetches with the agent token; the connector fetches through the host-authenticated command attachment route. The route serves only a file named by a command placed on that host and applies the target agent's attachment access. The device never receives an agent token.

Scheduled commands also preserve placement: the cron scheduler stamps `metadata.runtime_host_id` through `choruz_session::runtime_host_metadata`, so a remote agent's job runs on its device.

## Alternatives considered

**Execute outbox commands on the device.** Rejected because these are control-plane writes requiring an operator session and the central database, neither of which belongs on a runtime device ([unified runtime devices](2026-09-03-unified-runtime-devices.md)).

**Pull the outbox over the host link.** Rejected because the controller does not know when a device writes a command. Polling every remote workspace adds a round trip per binding per tick; the device already knows when to push.

**Add a `remote_outbox_command` table.** Rejected because the handler's ordering, claim-by-rename, stale claim recovery, and `share_file` resolution all operate on a directory. A table would still need a second handler or filesystem materialisation.

## Consequences

Remote `share_file`, `provision_agent`, `create_group`, `set_cron`, `task_*`, and terminal-DM `send` commands behave like local commands. Transport retries are idempotent for the lifetime of the mirror: losing an HTTP acknowledgement does not publish or execute an accepted command again. This does not change the pipeline's existing stale-`.processing` recovery after a command has been claimed.

The mirror retains one small accepted receipt per command and grows with its `results/` envelopes; removing the runtime directory removes both. An oversized `share_file` target is not shipped, so its command reports the missing file as a local command would. Commands left on a former device after a binding move are refused and dropped with a warning.

## Testing

- `crates/choruz-host-runtime/src/outbox.rs`: collection order and attachment bytes, symlink confinement, mirror placement, shipment count and byte limits, spooling, accepted-retry idempotency, and conflicting name reuse.
- `services/choruz-connector/src/main.rs`: permanent refusal is dropped and transient failure is retried.
- `services/choruz-pipeline/src/outbox_handler/tests.rs`: a command with a pending remote receipt is not processed; remote provisioning preserves device placement.
- `services/choruz-pipeline/src/outbox_watcher.rs`: remote terminal and headless mirrors are drained.
- `services/choruz-api-gateway/src/tests/runtime.rs`: the host-authenticated shipment and attachment routes enforce host placement, tokens, mirror layout, and file access.
- `crates/choruz-host-runtime/src/inbox.rs` and pipeline tests: attachment staging is confined to the target workspace; scheduled commands preserve the remote host stamp.
