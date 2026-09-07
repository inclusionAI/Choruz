# Agent Note: Rate-limit retry guidance follows the window

Status: implemented

## Problem

A fixed one-second retry hint sends callers back before a full minute's quota
has capacity. Merely checking that a request returns RateLimited misses this
incorrect guidance.

## Decision

The in-memory and database service entry points share one sliding-window
calculation. It removes expired accepted hits and computes the remaining time
until the oldest hit expires, rounding up to milliseconds. Rejections do not
consume quota. The gateway rounds that duration up to HTTP Retry-After seconds.

## Alternatives considered

**Return sixty seconds for every rejection.** Safe but unnecessarily delays a
caller whose oldest hit is about to expire.

**Maintain separate calculations.** Both entry points implement the same quota
contract; separate arithmetic can drift. The existing locks and per-principal
maps remain at their owners.

## Consequences

Controlled timestamps test expiry, decreasing waits and sub-millisecond
rounding without sleeping. Entry-point checks verify that both service paths
use the shared calculation. A configured zero limit stays closed; no retry
can make that configuration admit a request. This does not add distributed
quota coordination or client-side automatic retries.
