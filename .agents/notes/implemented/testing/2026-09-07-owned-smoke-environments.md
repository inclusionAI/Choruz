# Agent Note: Own destructive smoke environments

Status: implemented

## Problem

Smoke runners reset a database and, for restore checks, replace attachment data. Resolving those targets from development configuration makes an ordinary test invocation capable of destroying working data. Independent browser contexts cannot isolate these process and filesystem targets.

## Decision

The four destructive host smoke entry points delegate to `infra/host/isolated-smoke.mjs`. It allocates a private configuration and runtime tree, selects available loopback ports, and propagates that configuration to child host scripts. Development configuration and inherited database URLs are not inputs. PostgreSQL must stop before cleanup removes the owned data; logs survive for failure inspection.

The existing [acceptance policy decision](../process/2026-09-04-behaviour-acceptance-evidence.md) remains active: it owns assertion quality and execution evidence, not the allocation mechanism. No active note is superseded.

## Alternatives considered

**Require a separate checkout manually.** This leaves safety dependent on a caller remembering the instruction and does not isolate inherited environment variables.

**Reuse the development database with unique row names.** Reset and restore operate below row ownership, so unique names cannot protect existing data.

## Consequences

Each smoke invocation pays for a private PostgreSQL initialization. Node is required by these entry points. Available-port selection does not reserve a listener through service startup; a competing external binder can cause a startup failure, but cannot redirect cleanup into another run's data. Browser build caches remain checkout-scoped; simultaneous web builds still require separate checkouts.
