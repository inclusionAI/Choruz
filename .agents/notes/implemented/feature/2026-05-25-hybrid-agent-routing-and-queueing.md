# Agent Note: Hybrid agent routing and busy-agent queueing

Status: implemented

## Problem

A group conversation wakes an agent only when a message carries `@agent`, `@all` or an explicit metadata target. That keeps token cost and agent loops bounded, but a long-running team task stalls whenever a human sends an untagged question or an agent posts feedback without naming the next responsible agent, even though the next action is obvious to a human reader. The router is a trigger router, not a workflow router: it knows nothing about which task a message concerns or who owns it. Separately, one agent runs one headless turn at a time, so a direct question to a coordinator that is half-way through a long CI turn waits silently, and nothing in the UI says that it is queued.

## Decision

`route_event` in `crates/choruz-router/src/router.rs` plans every group message through `plan_routes`, which consults routing sources in a fixed order: explicit targets, structured workflow metadata, the conversation's untagged-human policy, and finally each agent's own `AgentPolicy` (`all_messages`, `mentioned_only`, `manual`). Every agent member gets a `route_decisions` row whose `reason` and `policy_snapshot` (`routing_source`, `workflow_kind`, `task_key`, `target_role`, `coordinator_fallback_reason`, ...) say why it was triggered or skipped; the audit surface grows through those two columns rather than through new ones. Routing guarantees live in platform state (`conversation_runtime_policies`, `group_workflow_task`) and router code; prompts encourage good handoffs but are not relied on for them.

Busy agents keep the headless queue: `PgSessionStore::find_pending_commands` (`crates/choruz-session/src/store.rs`) returns only `pending` rows for agents with no `leased`, `started`, `heartbeating` or `retry_scheduled` command, and `run_dispatch_loop` (`services/choruz-pipeline/src/dispatch.rs`) leases an agent's whole FIFO backlog as one batch. Nothing interrupts, reorders or cancels an active run; visibility comes from a derived status endpoint instead.

## Routing order

1. Explicit targets win. `explicit_target_plan` evaluates every agent under `MentionedOnly` regardless of its own policy, so `@name`, a principal id, a `mention_aliases` entry, `@all`, and `metadata.turn_for` / `metadata.request_review_by` trigger exactly the named agents and skip the rest with `explicit_target_not_selected`. A metadata target that matches no member yields `explicit_target_not_found` for everyone and does not fall through to workflow routing.
2. `metadata.workflow` on a group message (parsed by `parse_workflow_routing_event` in `crates/choruz-router/src/workflow.rs` into `kind`, `task_key`, `task_id`, `next_role`, `target_role`, `target_principal_ids`) is a task event. `workflow_target_plan` resolves the task through `MemberProvider::find_workflow_task` (`PgMemberProvider` in `services/choruz-pipeline/src/pg_member_provider.rs` accepts `task_id`, `task_key` or both and requires an active, visible assignee) and maps the kind to roles with `workflow_target_roles`: `task.started`, `task.blocked`, `task.idle`, `external_check.passed` and `human_input_needed` wake `coordinator`; `task.feedback`, `task.cleared` and `external_check.failed` wake `owner` plus `coordinator`; `task.ready_for_next_step` wakes `next_role`; `approval_required` wakes `approver` plus `coordinator`. The `owner` role is always the task's `assignee_principal_id` (`apply_canonical_owner_participant`); other roles come from `group_workflow_task_participant`. When the roles resolve to nobody, or the task is unknown, the configured coordinator is woken with `workflow_event_to_coordinator` or `workflow_task_missing_coordinator_fallback`; with no coordinator either, every agent is skipped with `workflow_task_not_found`.
3. An untagged message from a human sender follows `conversation_runtime_policies.untagged_human_mode` (`mentioned_only` by default, `coordinator_only`, `all_agents`; columns from `migrations/0024_hybrid_agent_routing.sql`, models `UntaggedHumanMode` and `ConversationRoutingPolicy`). `coordinator_only` wakes `default_coordinator_agent_id` alone (`untagged_human_to_coordinator`); when that agent is unset, is the sender, or is not an active member, `active_coordinator_agent_id` yields nothing and the message falls back to ordinary mention evaluation with `coordinator_not_configured`, `coordinator_is_sender` or `coordinator_unavailable` prefixed onto the reason. Agent senders never use this branch.
4. Everything else is `policy_fallback_plan`: per-agent `evaluate_trigger_with_candidates` from `crates/choruz-router/src/policy.rs`.

The router plans over agent members only, so no workflow kind pages a human; `human_input_needed` and `approval_required` reach the coordinator, whose role text (`AI_MANAGER_WORKFLOW_EXTENSION` in `apps/web/lib/agents/ai-manager-workflow-extension.ts`) is the only place agents are taught `metadata.workflow`. Messages that look like handoffs without metadata (`[DONE]`, `[BLOCKED]`, "feedback" next to a task-key-like token) are not routed: `workflow_text_marker_without_metadata` records a `workflow_text_marker` in the snapshot and `log_route_plan_observability` emits `workflow_text_without_metadata_skipped`, so the gap is diagnosable without being acted on.

Group provisioning sets the policy: `defaultEnableRoutingPolicy` in `apps/web/lib/groups/group-provisioning-runner.ts` writes `default_coordinator_agent_id` and `untagged_human_mode: "coordinator_only"` through `PUT /v1/runtime/policies/{conversation_id}` only when the template maps an explicit coordinator slot (`coordinatorRoleSlotId`) to an agent; display names are never inspected. `RuntimeStore::upsert_policy` (`crates/choruz-agent-runtime/src/policy.rs`) rejects a coordinator that is not a principal, and the column carries `ON DELETE SET NULL`.

## Shared task state

`group_workflow_task` (`UNIQUE (conversation_id, task_key)`), `group_workflow_task_participant` (`role_key` per principal) and the append-only `group_workflow_event` are the shared work items (0024); `agent_task` (`migrations/V008__agent_tasks.sql`) stays an agent-private planning surface. `send_to_group` in `services/choruz-pipeline/src/outbox_handler.rs` preserves `metadata.workflow` on the `conversation_events` row, and `process_group_send_workflow_metadata` appends a `group_workflow_event` for it through `DbService::append_group_workflow_event_for_conversation`. The channel kanban board (`migrations/0025_channel_kanban_board.sql`) builds on the same table, adding `assignee_principal_id` and the `todo`, `in_progress`, `blocked`, `in_review`, `done` status set the router filters on; see the [channel tasks board note](2026-05-29-channel-tasks-board.md).

## Busy-agent queue and visibility

`find_pending_commands` orders candidates by a per-agent `ROW_NUMBER()` so an idle agent's first message is never starved behind another agent's backlog; `idx_agent_commands_pending_fair` (`migrations/V022__idx_agent_commands_pending_fair.sql`) is the partial index for exactly that window. Dispatch groups the rows by `agent_id`, builds one `[choruz-batch]` prompt with `build_batched_prompt` when the group holds more than one command, and takes the group with `assign_batch_leases` in one transaction. Retries are bounded by `max_attempts`, default 3 (`migrations/V020__reduce_command_retry_budget.sql`).

`GET /v1/conversations/{conversation_id}/runtime-status` (`services/choruz-api-gateway/src/handlers_runtime_status.rs`; humans only, `last_error` passed through `redact_sensitive_text`) derives `AgentRuntimeStatus` with `PgSessionStore::list_runtime_status_for_agents`: `busy` when a `leased`, `started` or `heartbeating` command exists, `queued` when only `pending` or `retry_scheduled` rows exist, `idle` otherwise, plus `active_command` with `lease_age_seconds`, `queued_count` and `last_error`. `apps/web/components/runtime/runtime-status-panel.tsx` renders it in the detail panel with the banner "<agent> is busy. New messages will wait behind N earlier turns." The frontend never reads `agent_commands` directly.

## Alternatives considered

- **Wake every agent on untagged messages** (`all_agents` as the default): kept as an opt-in mode and as `@all`, but rejected as the default because it multiplies token and process cost per message and invites split-brain answers; one coordinator owns triage.
- **Rely on prompt etiquette for handoffs** (tell agents to always mention the owner): rejected because the observed group stalled precisely when an agent forgot the mention; the guarantee has to live in router state.
- **Text-pattern routing on `[DONE]`, `[BLOCKED]` and feedback markers**: considered as a compatibility fallback but not enabled, because there is no policy gate to put it behind and false positives would be undiagnosable; markers are recorded in `policy_snapshot` only.
- **Reuse `agent_task` as the shared task source**: rejected because that table is populated from per-agent task tools and is a private planning surface; a separate `group_workflow_task` keeps ownership and participants shared and board-compatible.
- **Suppress a workflow event whose task is unknown**: rejected in favour of the coordinator fallback so work keeps moving while the missing state stays visible in `route_decisions`.
- **Infer the coordinator from display names such as "manager"**: rejected; only an explicit template slot resolved at provisioning time sets `default_coordinator_agent_id`.
- **Interrupt, cancel or prioritise queued work for a busy agent**: rejected for this decision because interrupting a headless CLI turn risks session corruption and partial work, and priority queueing changes FIFO expectations and can starve older tasks; visibility is the substitute.
- **Bind the web UI directly to `agent_commands`**: rejected because the table is an internal state machine; the derived endpoint is the stable product contract.
- **Notify humans on ordinary workflow events**: rejected; only `human_input_needed` and `approval_required` are meant to reach a person, and even those go to the coordinator rather than paging anyone.

## Consequences

- Untagged human questions in a coordinated group reach exactly one agent, and every skip carries a reason an operator can query in `route_decisions`.
- The coordinator is a bottleneck and a single point of misrouting by design; explicit mentions and `@all` are the escape hatch and always win.
- Workflow routing works only for tasks that exist in `group_workflow_task` with a visible assignee; an agent that emits freeform text without `metadata.workflow` gets audit markers, not routing.
- A human message to a busy agent waits behind the whole active turn; the UI states this but cannot shorten it.

## Testing

- Router: `explicit_mention_wins_over_workflow_metadata`, `explicit_metadata_target_wins_over_workflow_metadata`, `missing_explicit_metadata_target_does_not_fall_through_to_workflow`, `untagged_human_message_routes_only_to_configured_coordinator`, `untagged_agent_message_does_not_use_human_coordinator_policy`, `unavailable_coordinator_policy_falls_back_to_mention_only_with_audit_reason`, `workflow_event_kinds_route_to_expected_agent_roles`, `owner_workflow_event_uses_canonical_assignee_over_stale_owner_participant`, `workflow_missing_task_falls_back_to_coordinator_or_skips`, `workflow_text_without_metadata_skip_has_audit_marker` and `rollout_scenario_routes_coordinator_workflow_feedback_and_at_all` in `crates/choruz-router/src/router.rs`; `parses_workflow_metadata_fields` in `workflow.rs`.
- Policy persistence: `policy_defaults_and_upsert_work` and `policy_rejects_invalid_default_coordinator` in `crates/choruz-agent-runtime/tests/runtime_store.rs`.
- Queue: `test_find_pending_commands_gives_idle_agents_a_fair_first_slot`, `test_find_pending_commands_coalesces_per_agent`, `assign_batch_leases_rolls_back_when_any_member_is_not_pending`, `assign_batch_leases_same_session_uses_one_epoch` and `test_runtime_status_for_agents` in `crates/choruz-session/tests/integration.rs`; the `batched_prompt_*` tests and `grouping_by_agent_id_distributes_commands_correctly` in `services/choruz-pipeline/src/dispatch.rs`.
- Status endpoint: `runtime_status_api_allows_workspace_humans_and_redacts_errors` in `services/choruz-api-gateway/src/tests/`.
- Provisioning: the coordinator-slot cases in `apps/web/lib/groups/group-provisioning-runner.test.ts`.

## Related

- [message-pipeline](../../../../docs/subsystems/message-pipeline.md) (the loops, leases and retry budget), [agent-protocol](../../../../docs/subsystems/agent-protocol.md) (the envelope and `metadata.workflow`), [choruz-agent-runtime](../../../../docs/subsystems/agent-runtime.md) (`conversation_runtime_policies` and the runtime-status route).
- [Per-turn roster injection](../architecture/2026-08-18-per-turn-roster-injection.md) supplies the `roster:` and `your_tasks:` envelope fields a coordinator uses to name owners.
- History: the [archived RFC](../../archived/feature/2026-05-25-hybrid-agent-routing-and-queueing.md) that weighed these alternatives.
