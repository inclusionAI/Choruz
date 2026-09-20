# Agent Note: Delayed sync acknowledgement coverage

Status: implemented

## Problem

A device can acknowledge an older page after another message commits. Tests that acknowledge every page immediately cannot establish whether that message remains replayable.

## Decision

The existing authenticated WebSocket integration test commits a second message before acknowledging the first page, observes its delivery, then acknowledges only the first page and reconnects. It checks the durable resume cursor and the second message's identity in the replay. PostgreSQL and a port-zero listener are owned by the test; received frames establish ordering without sleeps. The protocol itself is unchanged.

## Alternatives considered

**Change acknowledgement to the current feed head.** The client has not necessarily applied that head. Advancing to it could discard unacknowledged work on reconnect.

**Add a separate mocked socket suite.** Extending the existing production-path owner preserves authentication, persistence and replay coverage without duplicating its database setup.

## Consequences

This covers a committed message arriving between page delivery and acknowledgement. It does not establish transaction commit ordering for concurrent database writers or every network-failure interleaving. The [sync feed reference](../../../../docs/subsystems/sync-feed.md) owns the cursor contract.
