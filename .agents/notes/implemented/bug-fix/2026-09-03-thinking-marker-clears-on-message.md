# Agent Note: Clear thinking markers from message senders

Status: implemented

## Problem

The group-chat thinking indicator was cleared only after a reply sender
matched the client-side Agent roster. A message may arrive before that roster
refreshes, particularly for an imported session, leaving a visible reply and
its thinking marker together until the UI timeout expires.

## Decision

`apps/web/components/chat/chat-app.tsx` clears a thinking marker for every
received message sender, on both the WebSocket and recovered-history paths.
`apps/web/lib/messages/thinking.ts` owns the sender-ID extraction so the
behaviour has a focused regression test.

## Alternatives considered

**Wait for the roster refresh.** This keeps the stale indicator visible after
the reply has already reached the user and does not improve correctness.

**Add a server-side typing lifecycle.** That would provide richer semantics,
but is unnecessary to make the existing reply-driven indicator truthful.

## Consequences

Any message sender can attempt to clear a marker, but unmarked senders are a
no-op. The existing 120-second timeout remains only as a recovery path when no
reply reaches the client.

## Testing

`apps/web/lib/messages/thinking.test.ts` covers a sender absent from the
client-side roster and de-duplicates recovered message pages.
