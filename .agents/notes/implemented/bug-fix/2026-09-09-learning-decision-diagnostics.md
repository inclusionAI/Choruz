# Agent Note: Learning decision diagnostics

Status: implemented

## Problem

A background check can fail without creating a revision. A boolean content-review result cannot distinguish a rejected proposal from a response that violates the review contract, and an empty escalation list does not explain which recurrence condition failed.

## Decision

The existing audit store records each learning check and its device calls with a shared trace identifier and per-call identifiers. Start records precede calls; completion records contain duration and response fingerprints, while failures retain a closed error category without unstructured device error text. Revision validation links back to that trace and preserves the structured reviewer report, individual review predicates and per-observation escalation decisions. The decision predicates remain the activation authority; diagnostics do not relax them.

The [background-learning decision](../feature/2026-09-08-background-experience-learning.md) still owns source privacy and revision fencing. Audit records contain no TerminalSpec, account configuration, raw source window or reconstructed prompt. Model-call input fingerprints and revision source references link to the existing content owners instead of creating a second transcript archive.

## Alternatives considered

**Only enrich the final revision.** This loses failures before a revision exists and cannot distinguish an interrupted call from a completed one.

**Dump every request into service logs.** Requests contain account configuration and private source material. Metadata in the existing durable audit store preserves execution evidence without exposing those payloads in process logs.

**Infer rejection from a null instruction.** Evidence and addressed-problem validation can independently reject a returned instruction. Each predicate needs its own recorded result.

## Consequences

Audit writes add database operations to the background path, not foreground message handling. The revision and its commit audit share a transaction, so audit failure cannot leave an unreported activation. A failed pre-commit stage write aborts the check; failure to write the final check-completion audit is logged without changing a committed outcome. Process interruption can leave a start without a completion and does not prove model success. Stored history cannot recover a reviewer response discarded by an older deployment. These records explain decisions; they neither fix recurrence eligibility nor prove an intervention improved research.
