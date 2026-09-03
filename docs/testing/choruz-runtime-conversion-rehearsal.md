# Choruz Offline Conversion and Rehearsal

This is a breaking, offline operator procedure. Stop every legacy writer before
inspecting, copying, draining, discarding, or converting anything. Take and
verify PostgreSQL and filesystem backups first. Choruz never reads, clears,
renames, or converts legacy state at startup; this guide does not authorize an
automatic conversion helper for a live installation.

Use only a stopped, disposable copy. Do not point this procedure at a developer
checkout's `.runtime`, `.choruz-runtime`, a shared database, a registered user
worktree, a customer backup, or secrets. Roll back only by restoring the
verified backups while all writers remain stopped; do not reverse-rename a
partially converted copy.

## Required offline sequence

1. Stop every legacy gateway, pipeline, desktop process, timer, scheduler, and
   terminal driver. Record PIDs, listening ports, service-manager state, queue
   ownership, database identity, runtime root, `git worktree list`, and backup
   checksums. Refuse if a writer remains or source and target owners share a
   port, queue, workspace, attachment tree, ref, service label, or marker.
2. Make PostgreSQL and filesystem backups. Verify every dump and file checksum
   before touching the copied state, then rehearse restoring both into a fresh
   disposable location before conversion.
3. Inventory each source value below. If a source and Choruz identity collide,
   abort before mutation, restore the backup, and resolve the collision
   manually. Never choose a winner or overwrite either value.
4. With writers still stopped, choose and record one Maildir policy: **drain**
   every command to its normal result/evidence destination, or **discard** the
   queue into retained operator evidence. Do not rename an active queue, and do
   not silently delete queued commands.
5. Convert only the disposable copy. Regenerate supported artifacts from
   Choruz-only configuration. Recheck that no legacy identifier remains in the
   converted fixture, then preserve the backup until validation is complete.

## Reviewed old-to-new inventory

| Surface | Legacy value to inspect | Choruz-only value | Offline action and evidence |
| --- | --- | --- | --- |
| Environment and env files | `ECHAT_*`, `.echat*` env files | `CHORUZ_*`, `.choruz*` files | Copy only reviewed non-secret values into the copy; reject source/target file collisions. |
| PostgreSQL database and schema | `echat` database/user; `echat_outbox`, `echat_commands`, `echat_conversation_id`, `idx_bridge_mappings_echat_conv` | `choruz` database/user; `choruz_outbox`, `choruz_commands`, `choruz_conversation_id`, `idx_bridge_mappings_choruz_conv` | Restore backup into the disposable database, then apply the required forward migration. |
| Runtime paths and workspaces | `.echat`, `.echat-inbox`, `.echat-outbox`, `.echat-team.json` | `.choruz`, `.choruz-inbox`, `.choruz-outbox`, `.choruz-team.json` | Rename only the copied tree after stopped-writer and collision checks. |
| Git refs, worktrees, bootstrap | `echat/<session>`, old registrations, `.echat*` bootstrap markers and rebootstrap backups | `choruz/<session>`, reviewed registrations, `.choruz*` markers and backups | Snapshot refs and `git worktree list`; refuse duplicate refs, registrations, or markers before changing the copy. |
| Browser cookies and storage | `echat_session`, `echat:group-provisioning-job:*`, `echat_`/`echat-` keys | `choruz_session`, `choruz:group-provisioning-job:*`, `choruz_`/`choruz-` keys | Prefer a clean profile. Inspect a copied profile offline only; do not carry unreviewed cookies or storage. |
| Desktop bundle and application data | `com.echat.*` and every existing desktop profile/data location | `com.choruz.desktop` | `apps/choruz-desktop/tauri.conf.json` verifies the current bundle identifier. The shell writes its log beneath the OS data directory plus `choruz`; its source uses `dirs::data_dir()` rather than a hard-coded platform path. macOS container, Application Support, Windows AppData, and Linux XDG locations are therefore **operator-supplied paths** to record from the installed legacy bundle before copying; do not invent or bulk-rename them. |
| systemd and launchd | `echat-api-gateway`, `echat-pipeline`, `echat-web-app`, `com.echat.*` | `choruz-api-gateway`, `choruz-pipeline`, `choruz-web-app`, `com.choruz.*` | This slice does not enable, disable, or validate service managers; record them for the deferred 8.8–8.10 matrix. |
| API and provisioning contracts | `/api/echat/*`, `echat_*` operation IDs, `x-echat-*` headers | Choruz routes, `choruz_*` IDs, `x-choruz-*` headers | Reconfigure private consumers separately; no redirect or fallback is permitted. |
| MIME types | `application/vnd.echat.channel-task+json` | `application/vnd.choruz.channel-task+json` | Regenerate consumer configuration from the target contract. |
| Bridge files, configuration, keys | `echat-bridge` filenames/configuration and `echat_*` keys | `choruz-bridge` filenames/configuration and `choruz_*` keys | Rebuild a copied bridge configuration; bridge consumer validation is deferred to 8.8–8.10. |
| Agent protocol and helpers | `ECHAT_SEND`, `echat-*` binaries, helper/protocol markers | `CHORUZ_SEND`, `choruz-*` binaries, helper/protocol markers | Regenerate helper files from Choruz templates in the disposable copy. |
| Metrics, logs, PIDs, queues, generated artifacts | `echat_*` metrics, `echat:` queue keys, legacy PID/log/backup/result names | `choruz_*` metrics, `choruz:` keys, Choruz PID/log/backup/result names | Drain/discard stopped queues explicitly; delete and regenerate copied output only after backup verification. |

## Reproducible disposable rehearsal

Run from a clean checkout with PostgreSQL command-line tools available:

```bash
bash ./infra/host/runtime_conversion_rehearsal.sh all
```

The script creates a new `mktemp` root and an isolated PostgreSQL cluster under
that root. It refuses derived paths outside the root, starts no repository
service, creates no registered worktree, never reads `.runtime` or
`.choruz-runtime`, and removes the fixture on exit. It exercises both explicit
Maildir policies (`drain` and `discard`) and creates only synthetic direct
terminal-session state, pipeline/group workspace state, Maildir `new`/`cur`/
`tmp`, result data, an attachment, runtime bindings, external-session
provenance, PostgreSQL schema/rows, Git session-ref/worktree snapshots, and a
bootstrap marker.

For each policy, the harness proves a running writer fails closed before queue
or filesystem conversion; checksums and restores PostgreSQL plus filesystem
backups; rejects a pre-existing target collision before mutation; records queue
handling; converts the fixture; scans the target for unintended legacy
identifiers; and restores the original fixture state. Its synthetic conversion
logic is test-only evidence, not a production migration feature.

The following remain intentionally outside this rehearsal: live service-manager
operations; real/private terminal state; webhook, bridge, SDK, and consumer
deployments; and the wider actor/driver validation matrix in Tasks 8.8–8.10.
The final repository URL, clean-clone check, remote changes, package publishing,
and external cutover remain outside this slice.
