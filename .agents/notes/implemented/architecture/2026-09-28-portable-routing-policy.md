# Agent Note: Separate routing policy from database adapters

Status: implemented

## Problem

The generic router already accepts membership and decision adapters, but its event and command types pull in PostgreSQL clients. Consumers cannot reuse routing policy without building the platform's database integration.

## Decision

`choruz-router`, `choruz-session` and `choruz-store` expose their existing policy, retry and serialized types without default features. Their default `postgres` feature includes the existing store implementations and durable CDC router loop. The platform keeps those defaults; there is one implementation of each rule and SQL operation.

This is a permanent dependency selection boundary, not a migration switch. Unlike [device runtime persistence](2026-09-19-runtime-without-platform-database.md), these packages explicitly own storage adapters. Their database-free mode does not execute leases, save commands, authorize membership or provide a background service. Consumers provide those capabilities or compose the existing platform adapters.

## Alternatives considered

**Copy policy into a standalone SDK.** Rejected because mention resolution, task routing and retry semantics would acquire competing owners.

**Move all adapters into application.** Rejected here because session and event persistence are already cohesive independently usable packages. Moving their SQL into the broader control-plane application would enlarge the dependency required to reuse that storage.

**Create new crates for the row types alone.** Rejected because optional adapter dependencies preserve the existing model owners without another package per set of records.

## Consequences

Default builds retain PostgreSQL routing and integration tests. A separate CI command tests the database-free configuration and checks its normal dependency tree. The runnable example emits commands but deliberately does not claim delivery or durability. Cargo can unify features when another dependency requests PostgreSQL, so the isolated build check is necessary.
