# Agent Note: Compose the shared host by required services

Status: implemented

## Problem

Consumers of background learning need its API and persistence without mandatory group-chat execution, a web server or Remote Control pairing. A separate learning daemon would duplicate lifecycle and storage ownership.

## Decision

`choruz start local` selects `choruz-server --api-only`. The same supervisor starts and monitors the selected backend children. An explicit `CHORUZ_DATABASE_URL` selects a caller-migrated external database; otherwise the existing embedded PostgreSQL owner performs setup. Shutdown affects only owned children and the owned embedded cluster.

The [complete bundle decision](2026-09-02-headless-linux-bundle.md) still governs full-host distribution and TLS selection. API-only bundles require the API child and migrations but not the pipeline binary. Reusing an already running host does not change its composition.

## Alternatives considered

**A second lightweight learning service.** This duplicates the worker, authorization and persistence paths instead of making their existing owner usable independently.

**Always start the pipeline.** This makes a consumer of API learning also run unrelated group-chat work and install its binary.

## Consequences

API-only operation cannot process queued group-chat commands. Operators explicitly choose the composition before startup and supply migrations for external databases. The compiled-host test replaces only the API executable and verifies readiness, database forwarding and SIGTERM cleanup; it does not claim to exercise database migrations or learning execution.
