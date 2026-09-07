# Agent Note: Online guests share their own Agents

Status: implemented

## Problem

A shared group needs Agents from both participants' installations. A guest-side history projection alone cannot dispatch the guest's Agent, and a successful DM on that device does not prove shared-group participation. Generic rejection logs also hide whether registration, durable delivery, execution or reply forwarding failed.

## Decision

Keep the [canonical owner](2026-09-06-online-groups-have-one-canonical-owner.md) for ordering, and reuse the existing pipeline on each execution device. An explicit guest registration creates a credentialless namespaced Agent on the host. The host confirms membership before the guest adds its real Agent to an internal local execution group. The human UI continues to use the existing Online history projection, not a second visible group.

Each reachable Agent workspace has its own internal execution group, input/output cursors and workspace-scoped peer identities. This includes Companies without granting a Company Agent access to the human's personal workspace. Stable message identities make retries idempotent. Peer identities provide attribution and mention names but the member provider excludes them from local execution. Local Agent bindings, credentials and private DMs remain on their device. Membership generations keep a delayed confirmation from reactivating a removed registration.

The API's structured lifecycle events connect shared delivery ids to local message ids and existing pipeline logs. Rejections carry a bounded reason; message text and credential material are not diagnostic fields. The existing workspace-chat, identity and transport notes retain their independent authority boundaries.

## Alternatives considered

**Use Remote Control permissions for group participation.** This would grant device-level authority when the user only invited someone to a group.

**Implement a second Agent executor inside Online.** It would duplicate harness startup, account binding, outbox handling and failure recovery. A local execution group uses the same pipeline as ordinary groups.

**Claim success on local registration alone.** A disconnected host or name collision would leave an apparently active Agent that cannot speak. Host confirmation owns the active state.

## Consequences

The canonical owner must be online for new shared messages. A guest Agent must be explicitly shared and have a visible runtime binding in a workspace accessible to the guest owner. Group-only text and allowlisted author labels cross the encrypted mailbox; runtime operations, files and login credentials do not. Removal stops forwarding and new membership-based dispatch, not unrelated work in the Agent's private sessions.

Two-store integration tests cover the real registration HTTP entry, protocol receiver, identity mapping, input projection, reply forwarding, deduplication and forbidden identities. The browser test uses the real local Worker and APIs with a paused runtime fixture; a separate two-device live smoke is required to verify actual CLI execution.
