# Agent Note: Share bounded browser execution between decision and reasoning agents

Status: proposed

## Problem

The browser workflow executor can replay reviewed semantic steps, but a missing target or an uncertain decision ends the run without asking the selected reasoning Agent for help. Successful browser drafts also lack a reviewed reuse policy. Users cannot distinguish a proposed decision from a confirmed browser action in the current receipt.

## Proposal

Extend the existing decision workflow and device browser adapter instead of adding another execution service. Keep device dispatch, workspace authorization, durable admission, cancellation and browser-session ownership in their current owners.

A run has an explicit goal, permitted actions and pages, independent outcome checks and bounded execution and handoff budgets. The decision provider chooses from fresh observations. The selected native Harness supplies requested text or a bounded recovery proposal when needed. Its proposal passes the same execution guards; it cannot expand authorization. An uncertain mutation is inspected, never blindly repeated.

Record proposed decisions, attempted actions, confirmed actions, handoffs and verification separately. Preserve provider model identity, usage and elapsed time without inventing prices. Page content is untrusted input, not permission to change the goal or execution scope.

Reuse follows an explicit standing authorization scoped to a reviewed workflow revision and device. Future task matching does not grant authority. Failed execution returns evidence to the native Agent; repair creates a new candidate version, which must be evaluated before activation. Existing program evaluation and learning own these transitions.

## Alternatives considered

**Run generated JavaScript or shell scripts with browser and provider credentials.** Declarative proposals and existing semantic browser operations keep execution authority separate from model output.

**Add a second agent service and its own run database.** Existing runtime dispatch and browser admission already own these responsibilities. A second owner would introduce divergent cancellation, authorization and outcome state.

**Automatically enable every successful draft.** One success does not prove applicability to new inputs or authorize future side effects.

## Acceptance criteria

- A real browser run demonstrates decision execution, a selected-Harness handoff, continuation and independent verification in the same owned session.
- An ordinary failed decision preserves evidence and respects cancellation; uncertain mutations are not automatically replayed.
- Reviewed reusable workflows can be matched within explicit standing authorization, with evaluated revisions and revocation respected.
- The execution view distinguishes proposals, actions, handoffs and verification, and reports measured usage and timing.
- Existing finite classification, extraction and routing programs use the same validation and evaluation owners rather than a parallel implementation.

## Risks

Browser content can contain hostile instructions. Native reasoning must not bypass the action and page guards. Recovery can duplicate side effects unless uncertain outcomes remain distinct from pre-action failures. Large observations, additional tabs and background reuse need bounded resource ownership and honest partial-result reporting.
