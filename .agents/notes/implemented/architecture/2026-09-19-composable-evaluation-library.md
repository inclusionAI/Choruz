# Agent Note: Separate evaluation policy from platform execution

Status: implemented

## Problem

Evaluation tasks, team specifications and reflective optimization are pure logic, but ownership inside the control-plane domain crate makes standalone consumers inherit unrelated chat and community types. A second standalone implementation would drift from production selection and holdout rules.

## Decision

`crates/choruz-evaluation` owns the existing evaluation, optimization and team modules. Platform consumers depend on it directly; the [community library](2026-09-19-community-library.md) uses its team type for solutions. There are no compatibility re-exports or alternate optimizer. The library depends only on serialization, while platform workers retain leases, authorization, model execution, persistence and activation.

This is a library boundary within the [modular monolith](2026-08-18-modular-monolith.md), not a new service. The [reflective search decision](../feature/2026-09-09-reflective-learning-optimization.md) still owns the algorithm and its fixed evaluation constraints.

## Alternatives considered

**Expose the whole domain crate.** It is already free of I/O, but couples evaluation consumers to unrelated platform contracts and prevents an independently scoped package surface.

**Copy the optimizer into a separate SDK.** Two owners for checkpoint and selection semantics would require parallel fixes and risk different production and standalone results.

**Move the queue into the library too.** Database leases, tenant authorization and activation are platform responsibilities. The existing action/checkpoint interface already allows a caller to supply execution and storage without a speculative scheduler abstraction.

## Consequences

Wire and persisted JSON remain unchanged; Rust callers use the owning crate directly. Existing invariant tests move with their implementation. The packaged deterministic example exercises the public search interface and checkpoint serialization outside the workspace; it does not establish live-model improvement. Other runtime and community boundaries remain separate work.
