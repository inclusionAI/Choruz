# Agent Note: Keep platform persistence out of the runtime library

Status: implemented

## Problem

CLI configuration and native session discovery share a crate with `RuntimeStore`, so a device-side or standalone consumer acquires PostgreSQL client dependencies even though it never accesses the platform database.

## Decision

`choruz-agent-runtime` owns the existing pure binding vocabulary, CLI helpers and native discovery. `choruz-application::runtime_store` owns the pool, binding operations, conversation policies and row decoding. Production callers select those owners directly; there is no database feature flag or compatibility re-export in runtime. The database-backed tests and binding audit tests follow their implementation into application.

This continues the [modular monolith](2026-08-18-modular-monolith.md) and preserves the [device-operation boundary](2026-09-04-device-operations-behind-runtime-host.md): devices discover native sessions, while the controller checks binding identity before updating a row.

## Alternatives considered

**Make PostgreSQL an optional runtime feature.** That permits a smaller build but keeps platform policy and SQL ownership inside the reusable device library, with feature-dependent public APIs.

**Create another persistence crate.** Application already owns platform data access. A new store framework or package would add structure without a new independently useful responsibility.

**Move native discovery into application.** It would make device consumers depend on platform persistence and reverse the intended dependency direction.

## Consequences

Runtime and its common dependency can be packaged without a database client. Application adds a one-way dependency on runtime types and native discovery. SQL, serialized fields, state transitions and device dispatch remain unchanged. This does not extract interactive PTY execution or the asynchronous learning worker; those responsibilities retain their existing owners.
