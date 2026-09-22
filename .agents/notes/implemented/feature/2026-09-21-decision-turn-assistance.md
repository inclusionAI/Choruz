# Agent Note: Keep decision assistance inside native turns

Status: implemented

## Problem

A selected finite-output program is useful to later tasks, but treating its output as a complete Agent reply would bypass native transcript continuity, tool permissions and group delivery. Background evidence disclosure alone also does not authorize disclosing each new foreground request.

## Decision

Require a separate turn-assistance setting and a current validated selection. The controller dispatches inference with current consent and checks selection again before accepting the result. Send only immutable evidence with the native command, never a delayed instruction to invoke the provider. A shared preparation function provides bounded proposals to the native Agent without replacing it. Provider failures and abstentions are observable handbacks, not task failures or completion claims. Pin the evaluated model identity and reject a changed identity.

## Alternatives considered

**Publish the selected answer directly.** A finite choice can be a useful subtask result without satisfying the complete user request; native execution remains responsible for tools and the final reply.

**Duplicate local and remote decision loops.** One preparation owner keeps cancellation and proposal semantics consistent across session, pipeline and connector consumers.

## Consequences

Assistance does not eliminate the native turn or establish token savings. Browser action execution requires its own observation, authorization and outcome checks. Foreground inference has a five-second budget. Structured sessions reserve before inference; accepted duplicates reuse the native result, and interrupted or expired reservations cannot submit a late result. A request already dispatched to the provider cannot be recalled.
