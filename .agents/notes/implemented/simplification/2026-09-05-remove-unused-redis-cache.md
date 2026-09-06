# Agent Note: Remove the unused Redis cache

Status: implemented

## Problem

The store crate exposed `RedisCache` and carried Redis client and pool dependencies without a production consumer. References to the cache module were limited to its definition, implementation, export, dependency declarations, package README and store subsystem documentation. The cache's silent failure behavior had no application owner or measured need.

## Decision

The store crate owns PostgreSQL event storage and CDC intake. It has no Redis cache module, export or dependency; the workspace dependency declarations and lockfile contain only dependencies used by retained code.

## Alternatives considered

**Keep the cache for future use.** An unconsumed implementation adds dependencies and a public API without improving a current path. A future cache needs a measured workload and an owning consumer before its failure and invalidation semantics can be chosen.

**Replace it with a cache abstraction.** No production consumer requires caching, so an abstraction would preserve speculative surface rather than remove it.

## Consequences

Production storage behavior is unchanged. Reintroducing a cache requires concrete consumer and performance evidence. Existing store tests and compilation of dependent crates validate that removal does not leave a caller behind; no replacement cache tests are needed.
