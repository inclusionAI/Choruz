# Agent Note: Deliver the verified artifact and recover failed activations

Status: implemented

## Problem

A green source-level CI run does not establish that a packaged application starts after extraction. Rebuilding during deployment, mutating active release links during packaging and deploying a moving branch can each run code different from the verified artifact. Mirrored repositories also need an explicit owner for shared cloud deployment.

## Decision

CI builds an immutable, commit-addressed archive and boots its API, pipeline and standalone web application against an isolated PostgreSQL instance. A separate delivery workflow authorizes only successful same-repository main-push runs, verifies the retained artifact, and publishes without replacing existing assets. PR artifacts never reach production credentials.

The designated repository's cloud-production environment owns the Cloudflare token. Gateway promotion checks storage migration state, pins a uniquely uploaded version and verifies the serving version and Online session route. Failure restores the exact prior version unless an intervening deployment makes rollback unsafe. Serialized, non-cancelling delivery and lifecycle records make the result diagnosable.

Device updates are opt-in. One release helper owns manifest validation, atomic link replacement, managed-service restart and health-checked recovery. Writable state is outside release directories. Packaging does not activate code.

## Alternatives considered

- Deploy a fresh build of main after CI: the shipped bytes and revision can diverge from the verified run.
- Share cloud deployment credentials with every mirror: multiple workflows can overwrite one production Worker independently.
- Roll back storage with code: a code version is not a safe inverse of a database migration. Schema changes need a separate compatibility review.
- Restart every paired device automatically: deployment would interrupt user-owned Agent sessions without a maintenance decision.

## Consequences

The hosted artifact has an explicit Ubuntu 24.04 x86_64 baseline, with Node and PostgreSQL as external prerequisites. It does not replace portable old-glibc bundles. Cloud deployment requires environment credentials and a current migration baseline; either missing prerequisite stops promotion. Recovery tests replace service-manager and Cloudflare control-plane boundaries, while package acceptance runs the real binaries, database and HTTP endpoints. Power-loss recovery, ongoing availability monitoring and storage rollback remain separate operational responsibilities. Procedures and exact entry points live in [verified deployment](../../../../docs/operations/deploy.md).
