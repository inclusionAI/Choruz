# Agent Note: Versioned decision-program trials

Status: implemented

## Problem

Program generation used to reserve only the corpus, permanently preventing a
changed builder or model from being evaluated against the retained objectives.

## Decision

Reservations now include the decision settings and actual builder/judge execution
configuration. Session IDs, timestamps and credentials are not fingerprint inputs.

Keep reservation, external-call state and lease fencing together. A completed or
uncertain configuration is not automatically retried; four configurations per
corpus bound repeated held-out comparisons. A legacy reservation without a
completed report remains uncertain rather than being treated as free budget.

## Consequences

The existing decision-trial owner regression proves changed configurations are
admitted, duplicate and uncertain attempts are suppressed, and the corpus budget
and revoked claims are enforced. The assembled worker regression verifies a
completed duplicate does not dispatch another device call.

## Alternatives considered

Deleting old reservations would allow unknown paid calls to repeat. Unlimited
configuration keys would remove the held-out search budget. Keep old evidence
and admit bounded, distinct execution configurations instead.
