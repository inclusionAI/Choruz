# Agent Note: Complete finite tasks without another native call

Status: implemented

## Problem

The optional fast program produced advice, but every turn still invoked the
native model. This increased calls rather than replacing inference for tasks the
evaluated program could answer.

## Decision

Keep advisory permission unchanged. A separate `complete_turns` setting permits
only an evaluated, applicable, pinned-model finite output to complete a turn.
Abstention, provider failure, changed selection or an unevaluated output retain
native fallback. A finite result does not execute tools or establish tool effects.

Use the existing structured-session reservation, submission and journal owners.
Keep finite items when native history is restored and supply their pending context
to native fallback. Headless and connector turns use the existing reply commit;
execution metadata records actual model, usage and avoided native inference.
Structured completions also append a deduplicated non-chat `runtime.decision`
event. The existing feedback reader includes it; normal message lists do not.

## Alternatives considered

Treating advice as completion would silently broaden existing consent. A second
chat history or scheduler would duplicate lifecycle and ownership rules. Reported
token usage cannot establish subscription savings, so no dollar estimate is made.

## Consequences

The finite program has bounded outputs, not arbitrary coding capabilities. Native
fallback may still run collaborators. Context size limits are explicit rather
than dropping pending answers. The existing owner tests cover native-call
avoidance, advisory/failure fallback, duplicate delivery, reopening, cancellation
and learning attribution. External model responses are fixtures, not quality
measurements.
