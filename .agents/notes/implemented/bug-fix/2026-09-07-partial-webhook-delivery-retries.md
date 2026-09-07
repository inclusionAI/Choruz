# Agent Note: Retain successful destinations during webhook retries

Status: implemented

## Problem

A signed webhook can fan out to several external channels. Marking the event complete before delivery loses failed destinations; retrying every destination duplicates successful deliveries.

## Decision

`services/choruz-bridge/src/webhook-server.ts` keeps successful platform/channel pairs in its in-memory replay entry. Concurrent delivery attempts return 503. A partial failure also returns 503, and a retry skips successful destinations and attempts the remainder. Only an entirely successful fan-out marks the entry complete.

## Alternatives considered

**Acknowledge before fan-out.** This suppresses retries even when a destination fails, hiding message loss behind a successful response.

**Retry the entire fan-out.** This repeats delivery to destinations that already succeeded.

## Consequences

The HTTP test exercises overlapping requests and one failed destination, then verifies the retry and completed duplicate response. The cache is bounded to 10,000 entries with a five-minute lifetime; it is not durable exactly-once delivery. Process restarts, cache eviction, expiry, or an external service accepting a message before reporting failure can still produce duplicates.
