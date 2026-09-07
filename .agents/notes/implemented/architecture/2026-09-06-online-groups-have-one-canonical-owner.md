# Agent Note: Online groups have one canonical owner

Status: implemented

## Problem

People on different installations need to participate in one group without
receiving the owner's device authority. Independent conversation replicas would
also duplicate message ordering and Agent execution.

## Decision

The owner's conversation remains canonical. An accepted invitation creates a
credential-free human principal in that conversation, not a Company member or
runtime host. Guest text and HTTP messages share persistence, event notification
and webhook dispatch; the existing pipeline remains the only Agent execution
owner. A separate local link id avoids confusing the two installations' records
with the cloud channel.
Invited display names do not reserve local usernames. A database constraint
keeps these marked guest principals credential-free; local username
lookup excludes them even when an invited name matches an existing account.
The owner's chat renders author names from bootstrap principals, not only the
Agent catalog. Guest humans retain human styling in messages and threads.
The display catalog includes removed members of authorized conversations so
revocation does not erase history attribution after reload. It is not an
assignment roster: current membership still gates tasks and message access.

The API process owns the Online connection and durable inbox/outbox. Browser
closure does not stop it. Relay receipt is acknowledged after inbox persistence;
application completion is tracked separately. Canonical history is projected as
text with author identity, type and sequence. Periodic guest cursors request
replay after mailbox expiry. Revocation serializes with history persistence.

## Alternatives considered

**Forward arbitrary HTTP over Remote Control.** A group invitation must not
authorize private conversations, account management or terminal operations.

**Replicate the full conversation database.** It introduces competing sequence
and Agent execution owners. A text projection has a smaller authority boundary.

**Let the modal own the socket.** Browser navigation would interrupt delivery
and lose in-flight state; the server owns both connection and durable queues.

**Keep author names only in browser memory.** A fresh page loses revoked guest
names. Historical membership supplies attribution without restoring access.

## Consequences

The owner must be online for new canonical messages and Agent replies. Guests
can read received history after removal. Attachments and oversized messages have
explicit placeholders; files and terminal output are not shared. Invitation
holders can read the group's history, so the UI requires private sharing.

PostgreSQL tests cover actor isolation, durable delivery identity, history replay
and revocation. Browser tests use real Worker/D1 and API/PG boundaries for
invitation, human and authenticated Agent messages, webhook receipt, reload and
removal. The Agent API fixture does not establish a real CLI turn; dual-device
CLI acceptance is a separate requirement.

## Related

- [Guest-owned Agent execution](2026-09-06-online-guests-share-their-own-agents.md) extends participation while retaining canonical ordering and group-only authority.
- [Online transport](2026-09-06-online-mail-is-not-runtime-control.md)
- [Workspace chat presentation](2026-09-06-online-groups-share-the-workspace-chat.md)
- [Data model](../../../../docs/data-model.md#online-group-storage)
