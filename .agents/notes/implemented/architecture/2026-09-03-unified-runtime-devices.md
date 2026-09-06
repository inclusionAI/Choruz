# Agent Note: Remote Control devices converge on Company runtime hosts

Status: implemented

## Problem

Remote Control knew a paired browser, while Company execution knew a `runtime_host`. Pairing computer B from computer A therefore opened B's Dashboard but did not make B selectable in A's Harness Accounts, Create Agent, or Import Sessions flows. Keeping both concepts independent would make every device feature choose which incomplete list it meant and would let account, workspace, and Agent placement drift apart.

## Decision

An existing Choruz Dashboard turns a Remote Control pairing into a Company runtime host during the same **Add device** flow. A first encrypted A-to-B session carries a second one-time Remote Control credential issued by A and an eight-digit `runtime_host_pairing` code for the selected Company. B runs `choruz-connector pair-relay` with those values on stdin. The connector completes the reverse B-to-A encrypted pairing, redeems the runtime-host code through that channel, stores only the resulting device credentials and host token in a mode-`0600` config, and remains available without an inbound port.

`runtime_host` is the only Company device identity. Harness accounts, Agent bindings, filesystem browsing, and native-session import all carry its id. The controller requests `filesystem.home`, `filesystem.list`, and `workspace_sessions.scan` over the device's host link ([one host link carries every request to a remote device](2026-09-04-host-link-for-remote-devices.md)). A remote import repeats the scan on that device immediately before mutation, stores `native_session_import.runtime_host_id`, and creates a terminal binding whose `config_json.runtime_host_id` places its terminal and later turns on the same connector. The local execution path remains an explicit **This computer** option and has no synthetic database host row.

The API Gateway supervises saved connector configs from `~/.choruz/connectors/` for its full process lifetime. It discovers configs installed after startup and restarts a connector after an unexpected exit; a per-config lock prevents duplicate processes during upgrades. Revoking the Company Machine invalidates the host token, so the connector can no longer claim commands, logins, or operations ([connector supervision](../bug-fix/2026-09-04-supervise-runtime-connectors.md)).

## Alternatives considered

**Keep Remote Control devices and runtime hosts separate.** Rejected because a successful pairing would still not identify where an account, workspace scan, or Agent executes, and each product surface would need its own reconciliation rules.

**Expose B's API with a public port or Cloudflare Tunnel.** Rejected because it adds network and deployment configuration to the three-step `install → choruz start → paste credential` workflow and exposes more than the existing end-to-end-encrypted relay.

**Proxy all device work through B's full remote Dashboard.** Rejected because A needs one Company-level device inventory that can mix A, B, C, and D. Opening an isolated dashboard per computer does not give Agent placement or cross-device account selection to A's Company.

**Copy Harness credentials or native session files to A.** Rejected because credentials and filesystem state remain owned by the machine where the Harness runs. The connector returns only bounded operation results and executes Agent turns in place.

## Consequences

A single pairing makes a remote computer usable throughout the controlling Company, and any number of computers can join independently. Workspace paths and session catalogs stay device-scoped; identical native session ids on different machines do not collide. The Cloud Gateway carries ciphertext only, but the controlled computer now keeps a persistent reverse connector and config until its Machine is revoked and the local file is removed. Filesystem browsing is intentionally limited to `HOME` or `CHORUZ_FS_BROWSE_ROOTS`, 500 entries per listing, an 8 MiB operation response, and short-lived leased jobs. The onboarding and reverse connector add a second pairing exchange, so integration coverage must retain both the encrypted wire tests and the database lease/import test.
