# Agent Note: Resume learning stages without repeating completed inference

Status: implemented

## Problem

An optional search or program-builder failure can discard completed source analysis and prevent the cursor from advancing. One serial worker also makes unrelated profiles wait behind a slow device, while a fixed claim deadline can reject results after several successful model calls.

## Decision

The gateway admits at most four concurrent learning profiles. Existing database leases retain per-binding exclusion across processes; the worker renews only a live, enabled generation every thirty seconds. Losing that reservation stops the workflow, rather than permitting a later publication under stale consent.

A private, bounded window continuation freezes the projected visible source, initial analysis inputs, task corpus and scoped execution context. Successful pure analysis, task-review and research calls checkpoint their outputs before proceeding. Keys include the frozen window, complete effective request, fixed procedures and policy generation. Transient terminal-session metadata is not execution identity; account, device, profile directory, model and binary configuration are. Recovery rechecks authorization and context before using the continuation.

Program-trial reservation keys include consent generation in the database owner. A completed external trial can restore its full outcome to the same automatic continuation after a revision-commit interruption; a later generation cannot reuse that automatic outcome. The corpus-wide four-attempt budget spans generations. Explicit owner selection of a validated revision retains its existing authorization and applicability checks. Observed model-stage failures have at most three automatic attempts per frozen request; unknown outcomes require renewed consent without automatic redispatch.

Search failure preserves observations and reviewed tasks with an explicit pending status but installs no unresearched guidance. An unavailable optional program builder records its own pending outcome without discarding core results. Pending windows do not consume their source cursor; restoring the dependency resumes the same workflow using completed model-stage results. The transaction that commits a final report also removes the continuation and stage results, so a visible outcome never coexists with a resumable window; a window whose source was already analyzed removes them on its own. Saving settings, disabling learning or selecting another revision cancels generation-scoped continuation data; deleting the policy cascades it. An interrupted dispatch has an uncertain outcome and requires the owner to save settings before authorizing another potentially charged attempt.

The [background-learning decision](../feature/2026-09-08-background-experience-learning.md) retains source membership and activation ownership. The [diagnostics decision](2026-09-09-learning-decision-diagnostics.md) retains metadata-only audit records; private continuation content is not copied into those logs or public community records.

## Alternatives considered

**Catch errors and advance the cursor.** This preserves a summary but silently abandons the unfinished intervention or program, so dependency recovery never completes the original work.

**Restart analysis from its source on every retry.** Repeated model spend is unnecessary, and a changing corpus or summary can change the questions mid-workflow. A frozen continuation preserves the actual execution context.

**Retry unknown dispatches automatically.** A transport failure does not prove the provider did no work or incurred no charge. Explicit renewed consent distinguishes recovery from duplicate spending.

**Extend one global timeout.** This neither isolates slow profiles nor preserves completed stages. Bounded concurrency and fenced renewal retain both properties without another scheduler.

## Consequences

Private storage temporarily includes the visible source projection and completed model reports, unlike revision history's summaries and references. Hidden native reasoning, credentials and TerminalSpec are excluded from the continuation. Each window is bounded to 1 MiB and belongs to an opted-in policy; completion or an owner cancellation removes it. Long-lived failures can be cancelled by disabling learning or saving its settings. This is not public trace publication or permission to weaken source validation.

The regressions replace model generation only: they cover failed search followed by restoration, unavailable builder followed by a final suitability outcome, successful-call reuse, cancellation fencing and a four-slot saturation barrier with an independently progressing fifth profile. These fixtures establish lifecycle behavior, not model quality or improved benchmark scores.
