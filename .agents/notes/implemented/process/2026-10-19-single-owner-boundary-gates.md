# Agent Note: Enforce named architecture boundaries

Status: implemented

## Problem

Shared implementations can drift when a new adapter copies policy or bypasses its owner. A procedural instruction to reuse existing code does not reject a reintroduced raw browser fetch, private executor or retired projection module.

## Decision

The [architecture boundary contract](../../../../docs/testing/architecture-boundaries.md) combines an executable source gate with independent consumer tracing. `scripts/check_architecture.py` owns narrow lexical rules and retired paths. Static checks runs the gate and its negative-control tests for every non-documentation change. Ownership evidence is part of implementation PRs and the pre-simplify audit; the audit distinguishes semantic policy from intentional adapter differences.

The Rust test step supplies its read-only, ephemeral GitHub job token through `GITHUB_TOKEN` to the existing PostgreSQL downloader. The downloader remains the installation owner; no second download implementation, personal credential, test skip or retry is added. Step scoping avoids making the token a workflow-wide environment value.

## Alternatives considered

- **Only expand skill prose** — rejected because it does not mechanically prevent known bypasses.
- **Reject all similar code or every process spawn** — rejected because provider protocols, leases, transport and standalone library contracts legitimately differ.
- **Treat a green scan as proof of one implementation** — rejected because renamed equivalent code, aliases and transaction ordering require semantic inspection and behavioral evidence.

## Consequences

Known structural regressions fail without booting the product. Rule changes require a rejecting fixture and legitimate-use coverage; rules have no violation baseline or inline suppression. Owner moves require caller and rule updates together. New business capabilities outside the checked seams still require an independent ownership decision. The gate does not certify all repository architecture or replace persistence-failure and device-dispatch tests.
