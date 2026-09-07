# Agent Note: Online groups share the workspace chat

Status: implemented

## Problem

An invited group must behave like a conversation in the workspace, not a second
messenger inside an account-management dialog. Sharing a view must not make the
guest's projection look like a locally owned conversation to privileged APIs.

## Decision

The sidebar includes received Online groups beside local groups. A namespaced
link id selects the main pane and keys its draft. The Online transport adapter
uses the shared chat header, message list and composer. The Online dialog owns
only account and invitation management. Guest navigation never dispatches local
conversation, task, file or runtime requests; local bulk-management actions
exclude guest projections. This preserves the
[canonical owner](2026-09-06-online-groups-have-one-canonical-owner.md).

Received-history counts and bounded previews come from actor-scoped storage.
Read counts and the last selected group are browser-local, principal-scoped
preferences; opening received history clears that browser's badge. Drafts use
the existing composer store. Pending delivery remains visible until the owner
accepts the message. Revocation disables sending but retains received history.

The owner exports only allowlisted author display context: group owner, device,
Harness account name and driver. Account credentials, binding configuration,
runtime paths and arbitrary message metadata are not exported. Missing device
information is labelled as not shared rather than attributed to the guest's
computer. A group owner label is not a claim of individual Agent ownership.

## Alternatives considered

**Enlarge the Online dialog.** It preserves separate navigation and draft
behaviour, and still hides joined conversations when the dialog closes.

**Import guest links as ordinary local conversations.** Local mutation and
runtime routes would acquire ambiguous ownership. The display adapter keeps the
existing group-only authorization boundary explicit.

**Forward full Agent configuration for identity labels.** Display names do not
need profile paths or credentials. An allowlist bounds both exposure and payload.

## Consequences

Online and local chats share layout and input behaviour without sharing runtime
authority. Guest groups are account-wide, not memberships in the currently
selected local Company. Read indicators are specific to this browser; they are
not cross-device read receipts. Text-only transport restrictions remain visible
and attachments are unavailable in the guest composer. Participant attribution
comes from received messages, not a live full membership directory.

Browser acceptance uses real account, Worker, database and message APIs for
joining, switching local/shared groups, drafts, unread counts, reload and
revocation. Agent replies in that test use the authenticated Agent API; they do
not certify a new CLI execution path.
