# Agent Note: Automate browser reuse within standing permission

Status: implemented

## Problem

Per-draft live review and expiring reuse grants still require repeated human setup. Background work also stalls when asynchronous helper receipts require another user message. Replaying old tasks to validate automation risks duplicate external effects.

## Decision

The learning owner configures browser identity, exact pages and task scope once. Permission pins the binding's device/account configuration. The existing program builder produces declarative workflows from training evidence, excluding held-out records; the bound Agent discovers these drafts and requests only current tasks. The first requested task supplies live validation. Independent expected text belongs to that request, not the builder. Page checks must be absent before and present after execution.

The database serializes admission and a single durable dispatch claim against permission and learning generations. Configuration changes fence unclaimed work; stopping automation also cancels claimed work through the device. A crash after claiming leaves an uncertain receipt, never an automatic re-dispatch. A keyed request hash binds transient private inputs to a stable run ID. Failed or uncertain revisions cannot be retried by changing the run ID. The existing host executor owns fresh semantic target resolution, deadline, cancellation and session cleanup. Native assistance is limited to bounded target choices and named text generation; it cannot add actions, option values or checks. Results cannot overwrite cancelled receipts.

Durable completion receipts schedule the original conversation through the existing session command store. Stable command IDs prevent duplicate wakeups if the worker restarts before acknowledging a receipt. The UI contains permission, stop and activity history, not per-workflow execution forms. The old grant routes and consumers are removed; historical migration data remains.

This supersedes [per-run review](../../archived/feature/2026-09-21-reviewed-browser-workflows.md) and [bounded grants](../../archived/feature/2026-09-21-bounded-browser-reuse.md). Finite-answer programs retain their separate evaluation/selection contract.

## Alternatives considered

**Keep per-workflow confirmation.** It defeats the requested once-enabled automation. Standing human scope remains separate from advisory model matching.

**Replay historical traces for validation.** Recorded mutations may already have happened. Validate only while fulfilling a new requested task.

**Generate arbitrary scripts.** The declarative executor already owns targets, permissions and cancellation. Another runtime would duplicate authority.

**Poll inside a headless turn or wait for another human message.** Outboxes drain after return; persisted completion commands resume work without either deadlock or manual prompting.

## Consequences

Exact page and label constraints can reject changed sites; URL checks are not a network sandbox. Visible-text checks are limited transition evidence, not business correctness or generalization. Stop fences new admissions but cannot undo dispatched effects. Inspection and subsequent learning, not blind retries, handle uncertainty. Workflow instructions and receipts can contain private context; request input values are transient. Provider-reported tokens exclude native Harness usage. Changing device/account configuration requires renewed standing permission.
