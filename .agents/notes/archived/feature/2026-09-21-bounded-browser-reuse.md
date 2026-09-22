# Agent Note: Delegate reviewed browser work with immutable grants

Status: implemented
Archived: 2026-09-21

## Problem

Requiring a new human click for every familiar browser task prevents useful delegation. Automatically activating a generated draft instead confuses one successful example with permission for future mutations.

## Decision

Extend [reviewed execution](2026-09-21-reviewed-browser-workflows.md) with human-created, expiring grants. Approval binds an exact successful live run and owned learning revision to the execution configuration, independent checks, applicable tasks and a finite run budget. The grant stores input keys with cleared values. Current task values remain transient; a server-keyed request hash binds retries to their task and inputs.

The bound Agent can discover and consume grants, never create them. Applicability inference is advisory. The database serializes admission and revocation, rechecks the budget after inference, and rejects reuse after an unfinished or unsuccessful predecessor. The shared runtime-host browser executor owns effects, deadlines and cleanup. A revised workflow requires new live evidence and human approval.

## Alternatives considered

**Use matching confidence as authorization.** A model can misclassify a task; immutable human scope remains a separate prerequisite.

**Add a reusable-script runtime.** The existing declarative workflow, host transport and admission owner already enforce execution boundaries. Generated scripts would create another authority and cancellation path.

**Poll the outbox inside a headless turn.** Headless commands drain only after the turn returns. Durable results belong to later turns; the helper does not promise synchronous results or automatically schedule a continuation.

## Consequences

Revocation cannot undo an action already dispatched. Visible-text checks establish a limited page transition, not business correctness or generalization. Users must inspect failed or uncertain effects before approving further execution. Account/device configuration changes invalidate the grant rather than silently inheriting authority. Input values are cleared, but retained workflow instructions and checks can themselves be private and remain in the local database.
