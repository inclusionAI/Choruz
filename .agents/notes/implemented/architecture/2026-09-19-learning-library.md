# Agent Note: Compose fixed learning procedures with native execution

Status: implemented

## Problem

Analysis report contracts and fixed prompts share a module with native process management. Consumers cannot reuse the procedures without the host runtime, and host packaging reaches into root templates and the web test tree.

## Decision

`choruz-learning` owns report validation, fixed prompts and the analysis, task-admission and evaluation workflows. Its asynchronous adapters separate those procedures from evidence storage and native execution. Its optional [native CLI adapter](2026-09-28-portable-learning-cli.md) supplies `CliRunner`; host runtime provides the host evaluation adapter. Gateway workers retain authorization, source collection, scheduling, database leases and activation. `choruz-evaluation::dataset` owns curation and objective partitions independently of storage. Direct consumers use the owning package without a re-export shim; the gateway calls the same implementation rather than keeping a platform-specific learning algorithm.

Learning prompts are package assets. Native instruction fragments and the shared structured CLI fixture belong to host runtime. Web provisioning and tests read the same files rather than copying them. CI selects Rust owners through package dependencies and explicitly maps host assets and the shared fixture to their browser consumers.

This extends the [modular monolith](2026-08-18-modular-monolith.md), not the learning policy: prompt bytes, report formats, process restrictions and evaluation standards remain unchanged.

## Alternatives considered

**Expose native execution as the only learning entry.** Non-CLI consumers would need unrelated process and filesystem dependencies.

**Copy prompts or fixtures into packages.** Independent copies could silently diverge from product behavior.

**Add a second background scheduler.** Durability and activation would have competing owners; standalone callers instead own those policies.

## Consequences

Package archives contain their resources. Deterministic examples verify public entry points outside the repository, not model quality or live authentication. Runner implementers must enforce isolation and cleanup; the trait is not a sandbox. The repository layout map follows package ownership, while unchanged instruction fixtures preserve the model-visible contract.
