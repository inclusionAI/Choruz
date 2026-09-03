# Database Schema & Data Model

> **Last updated:** 2026-03-29 | **Author:** docs-dev | **DB:** PostgreSQL 16+

---

## Overview

Choruz uses PostgreSQL with two schema generations:

1. **Legacy schema** (migrations `0001`–`0008`) — core identity, conversations, messages, agent runtime
2. **Pipeline schema** (migrations `V001`–`V003`) — event-sourced message pipeline with routing, execution, and delivery

The pipeline schema is the primary path for event-sourced delivery. The legacy
`message` table remains for compatibility with older API paths and data.

---

## Entity Relationship Diagram (Logical)

```
                        ┌──────────────┐
                        │  principal   │
                        │ (user/agent) │
                        └──────┬───────┘
                               │
              ┌────────────────┴────────────────┐
              │                                 │
    ┌─────────┴──────┐                  ┌──────┴──────────────┐
    │ conversation   │                  │ agent_runtime_      │
    │ _member        │                  │ bindings             │
    └─────────┬──────┘                  └──────┬──────────────┘
              │                                 │
    ┌─────────┴──────┐                  ┌──────┴──────────────┐
    │ conversation   │                  │ agent_turn_leases   │
    └─────────┬──────┘                  └─────────────────────┘
              │
    ┌─────────┼──────────────────────────────────────┐
    │         │                                       │
    │  ┌──────┴──────┐   ┌───────────────────────┐   │
    │  │  message    │   │ conversation_events    │   │
    │  │ (legacy)    │   │ (pipeline)             │   │
    │  └─────────────┘   └───────────┬────────────┘   │
    │                                │                │
    │                    ┌───────────┼────────────┐   │
    │                    │           │            │   │
    │           ┌────────┴──┐ ┌─────┴─────┐ ┌───┴───┴──────┐
    │           │event_     │ │route_     │ │agent_        │
    │           │outbox     │ │decisions  │ │commands      │
    │           └───────────┘ └───────────┘ └──────┬───────┘
    │                                              │
    │                                    ┌─────────┼─────────┐
    │                              ┌─────┴───┐ ┌───┴───────┐ │
    │                              │agent_   │ │effect_    │ │
    │                              │results  │ │journal    │ │
    │                              └─────────┘ └───────────┘ │
    │                                                        │
    └─── receipt, audit_log, pending_replies, etc. ──────────┘
```

---

## Core Schema

### principal

Core identity table for human accounts and AI agents.

| Column | Type | Constraints | Notes |
|--------|------|-------------|-------|
| id | TEXT | PK | UUIDv7 |
| workspace_id | TEXT | NOT NULL | Tenant isolation |
| type | TEXT | NOT NULL | `human`, `agent` |
| name | TEXT | NOT NULL | Display name |
| avatar_url | TEXT | | |
| secret_hash | TEXT | | HMAC-SHA256 hash for agents |
| disabled | BOOLEAN | NOT NULL DEFAULT FALSE | Soft-disable |
| deleted_at | TIMESTAMPTZ | | Soft-delete |
| channel_visibility | TEXT | NOT NULL DEFAULT 'visible' | CHECK: `visible` or `internal`. Added by `0025_channel_kanban_board.sql`; `internal` agents (subagents, planners) are filtered out of the channel Tasks board roster and cannot be assigned channel-visible work |
| created_at | TIMESTAMPTZ | NOT NULL DEFAULT NOW() | |
| updated_at | TIMESTAMPTZ | NOT NULL DEFAULT NOW() | |

### conversation

Direct (1:1) and group chats.

| Column | Type | Constraints | Notes |
|--------|------|-------------|-------|
| id | TEXT | PK | UUIDv7 |
| workspace_id | TEXT | NOT NULL | |
| type | TEXT | NOT NULL | `direct`, `group` |
| name | TEXT | | Group name (e.g., `proj-team`) |
| description | TEXT | | |
| creator_id | TEXT | FK → principal(id) | |
| created_at | TIMESTAMPTZ | NOT NULL | |
| updated_at | TIMESTAMPTZ | NOT NULL | |

### conversation_member

Membership junction table. Access is based on active membership, not a role tier.

| Column | Type | Constraints | Notes |
|--------|------|-------------|-------|
| conv_id | TEXT | PK, FK → conversation(id) CASCADE | |
| principal_id | TEXT | PK, FK → principal(id) CASCADE | |
| joined_at | TIMESTAMPTZ | NOT NULL | |
| removed_at | TIMESTAMPTZ | | Soft-remove |

**Indexes:** `conversation_member_active_idx` — UNIQUE on (conv_id, principal_id) WHERE removed_at IS NULL

### message (legacy — being superseded)

Legacy message storage. New messages go to `conversation_events`.

| Column | Type | Constraints | Notes |
|--------|------|-------------|-------|
| id | TEXT | PK | UUIDv7 |
| conv_id | TEXT | FK → conversation(id) CASCADE | |
| sender_id | TEXT | FK → principal(id) | |
| content | TEXT | NOT NULL | |
| content_type | TEXT | NOT NULL | |
| metadata | JSONB | DEFAULT '{}' | |
| server_seq | BIGINT | NOT NULL | Per-conversation sequence |
| idempotency_key | TEXT | NOT NULL | Client dedup key |
| created_at | TIMESTAMPTZ | NOT NULL | |

**Indexes:**
- UNIQUE on (workspace_id, conv_id, sender_id, idempotency_key)
- UNIQUE on (conv_id, server_seq)

### agent_runtime_bindings

Links agent principals to runtime sessions (tmux/CLI).

| Column | Type | Constraints | Notes |
|--------|------|-------------|-------|
| id | TEXT | PK | |
| conversation_id | TEXT | FK → conversation(id) CASCADE | |
| agent_principal_id | TEXT | FK → principal(id) CASCADE | |
| driver_type | TEXT | NOT NULL | `claude_print`, `claude_terminal`, `codex_exec`, `codex_app_server`, `codex_terminal`, `pi_terminal`, `grok_terminal`, `opencode_terminal`, `acp`, `webhook_agent` |
| workspace_path | TEXT | NOT NULL | Agent's workspace directory |
| git_worktree_path | TEXT | | Isolated git worktree |
| external_session_id | TEXT | | tmux session ID |
| state | TEXT | DEFAULT 'idle' | `idle`, `running`, `paused`, `disabled`, `error` |
| last_event_cursor | BIGINT | DEFAULT 0 | Last event seen |
| last_acked_event_cursor | BIGINT | DEFAULT 0 | Last event acknowledged |
| in_flight_turn_id | TEXT | | Current active turn |
| last_error | TEXT | | Most recent error |
| config_json | JSONB | DEFAULT '{}' | Driver-specific config |

**Unique:** (conversation_id, agent_principal_id)
**Indexes:** conversation_id, agent_principal_id, state

### harness_account

One Claude Code or Codex login that agents of a company can run under, scoped to one device (`runtime_host_id` NULL is the API gateway's own computer). Credentials never enter this table: a `default` profile is the login the device already has, an `isolated` profile lives under `CHORUZ_HARNESS_ACCOUNT_ROOT/<id>/`.

| Column | Type | Constraints | Notes |
|--------|------|-------------|-------|
| id | TEXT | PK | UUID |
| company_id | TEXT | FK → company(id) CASCADE | |
| runtime_host_id | TEXT | FK → runtime_host(id) CASCADE | NULL for the gateway's own device |
| driver_type | TEXT | NOT NULL | `claude_terminal`, `codex_terminal` |
| name | TEXT | NOT NULL | Label shown in the UI |
| profile_kind | TEXT | NOT NULL | `default`, `isolated` |
| account_fingerprint | TEXT | | SHA-256 of the verified identity |
| subscription_type | TEXT | | Plan reported by the harness |
| status | TEXT | DEFAULT 'pending' | `pending`, `active`, `reauth_required`, `error`, `disabled` |
| models_json | JSONB | DEFAULT '[]' | Models the account verified |
| usage_json | JSONB | DEFAULT '{}' | Exact quota windows |
| last_error | TEXT | | Sanitized probe or login failure |
| probed_at | TIMESTAMPTZ | | Last successful verification |
| disabled_at | TIMESTAMPTZ | | Soft delete; the device keeps its credentials |

**Unique:** active `name` per (company_id, runtime_host_id, driver_type); one `default` profile per (company_id, runtime_host_id, driver_type); `account_fingerprint`
**Trigger:** `validate_runtime_binding_harness_account` on `agent_runtime_bindings` requires an `active` account of the same company, driver and host, and stamps `harness_account_name` and `harness_account_profile_kind` into `config_json`

### harness_account_login

One official browser sign-in for a harness account. A runtime host's connector claims a `queued` row; the API gateway claims a local one (`runtime_host_id` NULL) in the transaction that creates it. OAuth tokens and PKCE verifiers never enter this table.

| Column | Type | Constraints | Notes |
|--------|------|-------------|-------|
| id | TEXT | PK | |
| account_id | TEXT | FK → harness_account(id) CASCADE | |
| company_id | TEXT | FK → company(id) CASCADE | |
| runtime_host_id | TEXT | FK → runtime_host(id) CASCADE | NULL when the gateway runs the sign-in |
| driver_type | TEXT | NOT NULL | |
| state | TEXT | DEFAULT 'queued' | `queued`, `awaiting_browser`, `authorizing`, `verified`, `failed`, `cancelled`, `expired` |
| authorization_url | TEXT | | Official sign-in link |
| user_code | TEXT | | Codex device code |
| callback_code | TEXT | | Claude `code#state` pasted by the user, consumed once |
| error | TEXT | | Sanitized failure |
| created_by | TEXT | FK → principal(id) | |
| claimed_at, expires_at, completed_at | TIMESTAMPTZ | | 15-minute TTL from creation |

**Unique:** one open login (`queued`, `awaiting_browser`, `authorizing`) per account_id

### conversation_runtime_policies

Per-conversation agent behavior policies.

| Column | Type | Default | Notes |
|--------|------|---------|-------|
| conversation_id | TEXT | PK, FK → conversation(id) | |
| auto_mode | TEXT | 'mentioned_only' | `disabled`, `mentioned_only`, `metadata_only` |
| max_auto_turns | INT | 4 | Auto-turn limit |
| require_human_after_n_turns | INT | 4 | Human review gate |
| allow_agent_to_agent | BOOLEAN | FALSE | Cross-agent messaging |
| allow_file_write | BOOLEAN | TRUE | File system access |
| max_workflow_turns | INT | 20 | Workflow ceiling |
| default_reviewer_agent_id | TEXT | NULL | Optional reviewer agent principal |
| default_coordinator_agent_id | TEXT | NULL | Optional coordinator agent principal for hybrid routing |
| untagged_human_mode | TEXT | 'mentioned_only' | `mentioned_only`, `coordinator_only`, `all_agents` |

### group_workflow_task

Shared, group-visible workflow tasks. This is the storage layer behind the
**channel Tasks board (Kanban)** product surface described in
product requirements: every card a user or agent sees on the
board is one row in this table, and the silent `task_create` / `task_update` /
`task_transfer` outbox commands plus the `/conversations/:id/tasks` HTTP API
read and write here. The table is deliberately separate from `agent_task`,
which remains an agent-private execution/planning surface (Claude Code
`TaskCreate`, Codex `update_plan`, etc.) and never appears on the board.

The channel-tasks product surface belongs to the `kanban` Host/Client plugin
(see `docs/plugins.md` and PRD §12.1). When the plugin is disabled, rows still
exist as an audit trail but its HTTP routes and UI contributions are absent.

| Column | Type | Default | Notes |
|--------|------|---------|-------|
| id | TEXT | PK | |
| conversation_id | TEXT | FK → conversation(id) | Group conversation that owns the task |
| task_key | TEXT | | Human-readable key, unique within a conversation (e.g. `PROJ-12`) |
| title | TEXT | | User-visible title (must be non-blank / non-punctuation-only) |
| status | TEXT | 'todo' | CHECK constraint: `todo`, `in_progress`, `blocked`, `in_review`, `done` (set by migration `0025_channel_kanban_board.sql`) |
| assignee_principal_id | TEXT NOT NULL | FK → principal(id) ON DELETE RESTRICT | Canonical board assignee (visible owner); kept in sync with the `owner` row in `group_workflow_task_participant` |
| blocked_reason | TEXT | NULL | Optional free-text reason carried with `status = 'blocked'` |
| source_kind | TEXT | 'agent' | CHECK constraint: `agent` or `message`. `message` rows require `source_message_id` |
| source_message_id | TEXT | NULL | Message/event that introduced the task (required when `source_kind = 'message'`) |
| context_label | TEXT | NULL | Optional grouping label shown on the card |
| idempotency_key | TEXT | NULL | Agent-supplied key for `task_create` dedupe; unique per `(conversation_id, created_by)` when present |
| idempotency_payload_hash | TEXT | NULL | Hash of the payload that minted the idempotency key, used to detect conflicting re-issues |
| version | BIGINT | 1 | Monotonic per-task counter bumped by every mutation; surfaces in `group_workflow_event.resulting_version` |
| created_by | TEXT | FK → principal(id) | Optional creator (the principal that emitted `task_create` or the human who created from a message) |
| created_at | TIMESTAMPTZ | NOW() | |
| updated_at | TIMESTAMPTZ | NOW() | |

**Unique:** (conversation_id, task_key); (conversation_id, created_by, idempotency_key) when `idempotency_key IS NOT NULL`; (source_message_id, created_by) when `source_kind = 'message'`.
**Indexes:** (conversation_id, status); (assignee_principal_id); (conversation_id, assignee_principal_id). The `(conversation_id, task_key)` unique constraint backs router lookup and is the idempotency anchor for `task_create` (a duplicate `(conversation_id, created_by, idempotency_key)` returns `409 idempotency_conflict`).

The `description` column from the pre-MVP workflow-task surface was dropped by `0025_channel_kanban_board.sql`; channel cards carry only a title plus optional `context_label` and `blocked_reason`. Per-conversation monotonic `task_key` minting is backed by the companion `channel_task_sequence` table (also created in 0025).

### group_workflow_task_participant

Role assignments for a shared workflow task. Used by both the router (to
resolve workflow events like `task.feedback` to specific principals via
`role_key` values such as `coordinator`, `owner`, `quality_check`, and
`approver`) and the channel Tasks board (the visible "assignee" on each card
is the participant with the `owner` role; transfers via `task_transfer`
rewrite the owner row).

| Column | Type | Default | Notes |
|--------|------|---------|-------|
| id | TEXT | PK | |
| task_id | TEXT | FK → group_workflow_task(id) | |
| principal_id | TEXT | FK → principal(id) | Human or agent participant |
| role_key | TEXT | | Workflow role key |
| responsibility | TEXT | NULL | Optional role description |
| required | BOOLEAN | TRUE | Whether this participant is required for the task |
| created_at | TIMESTAMPTZ | NOW() | |
| updated_at | TIMESTAMPTZ | NOW() | |

**Unique:** (task_id, principal_id, role_key)
**Indexes:** (task_id, role_key), principal_id

### group_workflow_event

Append-only history for shared workflow tasks and auditable workflow metadata
seen by the platform. Events may have a null `task_id` when a workflow-looking
event cannot yet be attached to known task state. For the channel Tasks
product surface this is the **board history / activity log**: every status
change, transfer, and create observed through the outbox commands or the
HTTP API appends an event here, and the Tasks UI can replay it to show a
per-card timeline.

| Column | Type | Default | Notes |
|--------|------|---------|-------|
| id | TEXT | PK | |
| conversation_id | TEXT | FK → conversation(id) | |
| task_id | TEXT | FK → group_workflow_task(id) | Nullable for unresolved events |
| source_message_id | TEXT | NULL | Message/event that carried the workflow metadata |
| actor_principal_id | TEXT | FK → principal(id) | Optional actor |
| kind | TEXT | | Workflow event kind, for example `task.feedback`, `task.created`, `task.status_changed`, `task.transferred` |
| payload | JSONB | '{}' | Structured event payload |
| resulting_version | BIGINT | NULL | The `group_workflow_task.version` value the task held after this event (added by `0025_channel_kanban_board.sql`); lets the Tasks UI / replay consumers detect ordering gaps |
| created_at | TIMESTAMPTZ | NOW() | |

**Indexes:** (task_id, created_at), (conversation_id, created_at)

### Other Legacy Tables

| Table | Purpose | Key Fields |
|-------|---------|-----------|
| **receipt** | Read receipts per user per conversation | (principal_id, conv_id), last_read_seq |
| **audit_log** | Action audit trail | actor_id, action, target_type, target_id |
| **outbox_event** | Event delivery outbox | principal_id, event_type, payload, acknowledged_at |
| **agent_turn_leases** | Distributed turn-taking locks | binding_id, lease_owner, lease_until |
| **pending_replies** | Reply delivery tracking with dedup | binding_id, content_hash, status |

---

## Pipeline Schema (V001–V003)

### conversation_events

Append-only event log — the source of truth for the new pipeline.

| Column | Type | Constraints | Notes |
|--------|------|-------------|-------|
| conversation_id | TEXT | PK (composite) | |
| seq | BIGINT | PK (composite) | Per-conversation sequence |
| event_id | TEXT | UNIQUE | Global event ID |
| event_type | TEXT | NOT NULL | `message`, `reply`, `system`, `reaction`, `edit`, `delete` |
| sender_id | TEXT | NOT NULL | |
| content | TEXT | | Message body |
| content_type | TEXT | DEFAULT 'text' | |
| metadata | JSONB | DEFAULT '{}' | |
| client_msg_id | TEXT | UNIQUE (partial) | User message retry dedup |
| turn_id | TEXT | UNIQUE (partial) | Agent reply commit dedup |
| reply_event_id | TEXT | | Quote-reply target; for THREADED replies, the canonical thread root |
| created_at | TIMESTAMPTZ | NOT NULL | |

**Idempotency:** Two partial unique indexes on `client_msg_id` (user messages) and `turn_id` (agent replies) ensure exactly-once semantics.

**Threads (V018):** a threaded reply is a normal row with `reply_event_id` pointing at the thread ROOT (write paths canonicalize — never at another reply; threads are flat) and `metadata.thread = true` (JSON boolean; the shared SQL predicate `THREAD_FLAG_SQL` / Rust `ThreadFlags` in `choruz-store` is the single source of truth). `metadata.broadcast = true` additionally surfaces the reply on the main timeline ("also send to channel"). Quiet (non-broadcast) thread replies do **not** bump `conversation.total_msg_count` — thread unread is tracked per thread via `thread_read_receipt`. Legacy quote-replies (`reply_event_id` without the flag) are unaffected. Partial index `idx_conversation_events_thread (conversation_id, reply_event_id, seq) WHERE reply_event_id IS NOT NULL AND <thread flag>` backs the reply list, timeline rollups, and the unread LATERAL.

### thread_read_receipt (V018)

Per-principal thread read receipts — lazy rows, created on first view of a thread; cleared per thread (`POST /v1/conversations/{id}/threads/{root}/view`), not by viewing the conversation.

| Column | Type | Constraints | Notes |
|--------|------|-------------|-------|
| conversation_id | TEXT | PK (composite), FK → conversation ON DELETE CASCADE | |
| thread_root_id | TEXT | PK (composite), FK → conversation_events(event_id) ON DELETE CASCADE | Always the canonical root |
| principal_id | TEXT | PK (composite), FK → principal ON DELETE CASCADE | |
| last_read_seq | BIGINT | NOT NULL DEFAULT 0 | MAX reply seq at view time |
| last_read_at | TIMESTAMPTZ | | |

**Indexes:** `idx_thread_read_receipt_principal (principal_id, conversation_id)` for unread aggregation; `idx_thread_read_receipt_root (thread_root_id)` so the event-side cascade doesn't seq-scan.

### event_outbox

CDC source — rows copied from conversation_events for durable delivery.

| Column | Type | Notes |
|--------|------|-------|
| id | BIGSERIAL | PK, monotonic |
| aggregate_type | TEXT | Always 'conversation_event' |
| aggregate_id | TEXT | conversation_id |
| event_type | TEXT | Event type |
| payload | JSONB | Full event data |
| published | BOOLEAN | DEFAULT FALSE, set TRUE after CDC consumer processes |

**Index:** `idx_event_outbox_unpublished` — on (id) WHERE published = FALSE

### route_decisions

Audit trail for the Router's decisions on which agents to trigger.

| Column | Type | Notes |
|--------|------|-------|
| route_id | TEXT | PK (UUID) |
| message_id | TEXT | Source event |
| agent_id | TEXT | Target agent |
| decision | TEXT | `trigger`, `skip`, `error` |
| reason | TEXT | Human-readable explanation |
| policy_snapshot | JSONB | Policy state at decision time |

**Unique:** (message_id, agent_id) — one decision per agent per message

### agent_commands

Command state machine — tracks each agent execution request.

| Column | Type | Notes |
|--------|------|-------|
| command_id | TEXT | PK (UUID) |
| route_id | TEXT | UNIQUE, FK-like to route_decisions |
| session_key | TEXT | `{agent_id}:{conversation_id}` |
| turn_id | TEXT | UNIQUE — links to conversation_events |
| status | TEXT | See state machine below |
| attempt_count | INT | Current attempt number |
| max_attempts | INT | DEFAULT 3 |
| prompt | TEXT | The message content to process |

**State machine:**
```
pending → leased → started → heartbeating → succeeded → committed
                                          ↘ retry_scheduled → pending (loop)
                                          ↘ dead_letter (max attempts)
```

### session_registry

Tracks which executor node owns each agent session.

| Column | Type | Notes |
|--------|------|-------|
| session_key | TEXT | PK (`{agent_id}:{conversation_id}`) |
| executor_node_id | TEXT | Current owner |
| epoch | INT | Fencing token — incremented on owner change |
| status | TEXT | `idle`, `active`, `draining`, `dead` |

### agent_results

Stores the output of each execution attempt.

| Column | Type | Notes |
|--------|------|-------|
| turn_id | TEXT | Links to agent_commands.turn_id |
| attempt_id | TEXT | UNIQUE — one result per attempt |
| status | TEXT | `succeeded`, `failed` |
| content | TEXT | Agent's reply content |
| execution_duration_ms | BIGINT | Performance tracking |

### effect_journal

Idempotent tool call tracking for the Tool Gateway.

| Column | Type | Notes |
|--------|------|-------|
| tool_call_id | TEXT | PK |
| turn_id | TEXT | Parent turn |
| tool_name | TEXT | e.g., `send_message`, `read_file` |
| tool_input | JSONB | Call parameters |
| tool_output | JSONB | Result (NULL until completed) |
| status | TEXT | `pending`, `executing`, `succeeded`, `failed` |
| is_mutating | BOOLEAN | Whether this effect changes state |

### Other Pipeline Tables

| Table | Purpose |
|-------|---------|
| **mailbox_visibility** | Which agents can see which messages (routing output) |
| **client_cursors** | Per-client read position for cursor-based replay |
| **dead_letters** | Failed events that exhausted retries |
| **agent_policies** | Per-agent, per-conversation trigger config (`all_messages`, `mentioned_only`, `manual`) |

---

## Migration History

> Non-exhaustive — highlights only. The full ordered list is `migrations/*.sql`
> (the 0NNN series applies before the V-series; both in lexicographic order).

| Migration | Description | Type |
|-----------|-------------|------|
| `0001_init.sql` | Core tables: principal, conversation, message, receipt, presence, audit_log, outbox_event | Schema |
| `0002_app_snapshot.sql` | In-memory state snapshot storage | Schema |
| `0003_agent_runtime_bridge.sql` | Agent bindings, turn leases, runtime policies | Schema |
| `0004_runtime_terminal_mode.sql` | Add `claude_terminal` driver type | Alter |
| `0005_add_foreign_keys.sql` | Add missing FKs to runtime tables | Alter |
| `0006_add_max_workflow_turns.sql` | Add workflow turn limit to policies | Alter |
| `0007_add_missing_indexes.sql` | Performance indexes on audit_log, outbox, leases | Index |
| `0008_pending_replies.sql` | Reply delivery tracking with dedup | Schema |
| `0024_hybrid_agent_routing.sql` | Coordinator routing policy fields and group workflow task tables | Schema |
| `V001__message_pipeline_schema.sql` | Full pipeline: events, outbox, routing, commands, sessions, effects | Schema |
| `V002__agent_policies.sql` | Agent trigger policies | Schema |
| `V003__data_migration_legacy_messages.sql` | Migrate legacy messages → conversation_events | Data |
| `V018__message_threads.sql` | Message threads: partial thread index + thread_read_receipt | Schema |
| `V035__harness_accounts.sql` | `harness_account` and the runtime-binding validation trigger | Schema |
| `V036__remote_harness_account_logins.sql` | `harness_account_login` sign-in state machine | Schema |
| `V037__local_harness_account_logins.sql` | Nullable `runtime_host_id` for sign-ins the gateway runs itself | Alter |
| `V038__remove_app_snapshot.sql` | Remove the obsolete in-memory state snapshot table | Schema |
| `V039__company_multi_harness_accounts.sql` | `company.multi_harness_accounts`, the per-company multi-account switch (default off) | Alter |
| `V040__rename_remote_pairing_credential_hash.sql` | Name the Remote Control pairing hash after the opaque credential | Alter |

---

## Key Design Notes

1. **All IDs are TEXT** — UUIDv7 stored as text for readability and sortability
2. **Soft deletes** — `deleted_at` / `removed_at` timestamps instead of hard deletes
3. **Idempotency everywhere** — `client_msg_id` for user messages, `turn_id` for agent replies, `tool_call_id` for effects
4. **Append-only events** — `conversation_events` is never updated, only appended
5. **Outbox pattern** — `event_outbox` ensures at-least-once delivery via CDC polling
6. **State machines** — `agent_commands.status` tracks the full lifecycle with retry support
7. **Epoch fencing** — `session_registry.epoch` prevents stale executors from committing results
