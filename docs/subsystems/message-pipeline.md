# Message pipeline

The message pipeline is the single `choruz-pipeline` process that turns a persisted conversation event into agent turns and turns agent output back into conversation events: it claims `event_outbox` rows (CDC), routes them to agents, leases one batch of `agent_commands` per agent, runs a headless CLI turn, drains the agent's Maildir outbox, commits the reply, and fans events out over a compatibility WebSocket. A reader can use this page to follow one message end to end, to understand busy-agent queueing, leases and retries, and to operate the process (env vars, ports, the `rebootstrap` subcommand). Source: [`services/choruz-pipeline/`](../../services/choruz-pipeline/) plus the stage crates linked below.

## Owns

| Path | Role |
|---|---|
| [`services/choruz-pipeline/src/main.rs`](../../services/choruz-pipeline/src/main.rs) | Binary entry: `rebootstrap` subcommand or `pipeline::run_pipeline` |
| [`services/choruz-pipeline/src/config.rs`](../../services/choruz-pipeline/src/config.rs) | `PipelineConfig::from_env`, `validate`, `DISPATCH_HEARTBEAT_INTERVAL_SECS` |
| [`services/choruz-pipeline/src/pipeline.rs`](../../services/choruz-pipeline/src/pipeline.rs) | Spawns the task topology: CDC poller, router, dispatch, writer, lease monitor, retry scheduler, cron, outbox watcher, fanout, WS server |
| [`services/choruz-pipeline/src/dispatch.rs`](../../services/choruz-pipeline/src/dispatch.rs) | `run_dispatch_loop`, per-agent batching (`build_batched_prompt`), lease heartbeats, success/retry/dead-letter transitions |
| [`services/choruz-pipeline/src/executor.rs`](../../services/choruz-pipeline/src/executor.rs) | `ExecutorContext`, `execute_command`, headless CLI spawn, WAL recovery, outbox drain after each turn, error classification |
| [`services/choruz-pipeline/src/outbox_handler.rs`](../../services/choruz-pipeline/src/outbox_handler.rs) | Maildir claim/parse/dispatch of agent commands (`process_outbox_commands_with_stats`, `process_outbox_command_files`) |
| [`services/choruz-pipeline/src/outbox_watcher.rs`](../../services/choruz-pipeline/src/outbox_watcher.rs) | `run_outbox_watcher_loop`: drains outboxes of terminal and webhook bindings that have no headless turn |
| [`services/choruz-pipeline/src/cron_scheduler.rs`](../../services/choruz-pipeline/src/cron_scheduler.rs) | `run_cron_scheduler`: claims due `agent_cron_job` rows and inserts commands |
| [`services/choruz-pipeline/src/lease_monitor.rs`](../../services/choruz-pipeline/src/lease_monitor.rs), [`retry_scheduler.rs`](../../services/choruz-pipeline/src/retry_scheduler.rs) | Expired-lease handling, TTL sweep, exhausted-command dead-lettering, re-dispatch |
| [`services/choruz-pipeline/src/pg_notify.rs`](../../services/choruz-pipeline/src/pg_notify.rs) | `LISTEN choruz_commands` wake-up for the dispatch loop |
| [`services/choruz-pipeline/src/pg_member_provider.rs`](../../services/choruz-pipeline/src/pg_member_provider.rs), [`pg_result_store.rs`](../../services/choruz-pipeline/src/pg_result_store.rs), [`pg_event_source.rs`](../../services/choruz-pipeline/src/pg_event_source.rs) | PostgreSQL adapters for the router (`PgMemberProvider`, `PgDecisionSink`), writer (`PgResultStore`) and fanout (`PgEventSource`) |
| [`services/choruz-pipeline/src/instructions.rs`](../../services/choruz-pipeline/src/instructions.rs) | `ensure_claude_md`, `force_rewrite_bootstrap`, `run_rebootstrap_command` (see [agent-protocol](agent-protocol.md)) |
| [`services/choruz-pipeline/src/meta.rs`](../../services/choruz-pipeline/src/meta.rs) | `/healthz` and `/readyz` handlers |
| [`crates/choruz-store/src/cdc_poller.rs`](../../crates/choruz-store/src/cdc_poller.rs), [`event_outbox.rs`](../../crates/choruz-store/src/event_outbox.rs) | `CdcPoller`, `EventStore::claim_unpublished_entries`, `mark_published` (the store itself is owned by [store](store.md)) |
| [`crates/choruz-router/`](../../crates/choruz-router/) | `route_event`, `run_router_loop`, `evaluate_trigger`, `build_prompt`, `MemberProvider`, `DecisionSink` |
| [`crates/choruz-session/`](../../crates/choruz-session/) | `PgSessionStore`: `session_registry`, `agent_commands`, leases, epochs, retries, `dead_letters` |
| [`crates/choruz-executor/`](../../crates/choruz-executor/) | `SandboxManager`, `AdapterWal`, `CliAdapter` |
| [`crates/choruz-writer/`](../../crates/choruz-writer/) | `commit_result`, `run_writer_loop`, `ResultStore` |
| [`crates/choruz-fanout/`](../../crates/choruz-fanout/) | `FanoutGateway`, `ws_fanout_routes` (`GET /ws/fanout`) |
| [`crates/choruz-events/`](../../crates/choruz-events/) | `EventEnvelope`, `ConversationEventPayload`, `RouteDecisionPayload`, `AgentCommandPayload`, `AgentResultPayload` |

Tables: `event_outbox`, `mailbox_visibility`, `route_decisions`, `session_registry`, `agent_commands`, `dead_letters` ([`V001`](../../migrations/V001__message_pipeline_schema.sql)); `agent_cron_job` ([`V009`](../../migrations/V009__agent_cron.sql)); claim columns on `event_outbox` ([`V007`](../../migrations/V007__outbox_claim_lease.sql)); `max_attempts` default 3 ([`V020`](../../migrations/V020__reduce_command_retry_budget.sql)); `idx_agent_commands_pending_fair` ([`V022`](../../migrations/V022__idx_agent_commands_pending_fair.sql)).

Triggers: `notify_outbox_insert` on `event_outbox` and `notify_command_insert` on `agent_commands` are created by [`V004`](../../migrations/V004__notify_triggers.sql); [`V019`](../../migrations/V019__choruz_database_cutover.sql) replaces their bodies so they `pg_notify` on `choruz_outbox` and `choruz_commands`.

## Data

| Type | Where | Moves |
|---|---|---|
| `OutboxRow` (`id`, `published`, `claimed_by`, `claimed_at`, `claim_deadline`, `attempt_count`) | [`crates/choruz-store/src/event_outbox.rs`](../../crates/choruz-store/src/event_outbox.rs) | CDC poller to router over an `mpsc` channel |
| `RouteDecision` (`route_id`, `message_id`, `agent_id`, `decision`, `reason`, `policy_snapshot`), `RouteOutcome` (`trigger`/`skip`/`suppressed`), `AgentPolicy`, `AutoMode`, `ConversationRoutingPolicy`, `AssigneeRosterEntry`, `AssignedTaskHint` | [`crates/choruz-router/src/models.rs`](../../crates/choruz-router/src/models.rs) | Router audit rows and envelope inputs |
| `AgentCommand` (`command_id`, `route_id`, `session_key`, `agent_id`, `conversation_id`, `message_id`, `turn_id`, `status`, `current_attempt_id`, `current_epoch`, `attempt_count`, `max_attempts`, `prompt`, `metadata`, `next_retry_at`, `last_error`), `CommandStatus`, `InsertCommand`, `CommandStatusUpdate`, `Session`, `AgentRuntimeStatus` | [`crates/choruz-session/src/models.rs`](../../crates/choruz-session/src/models.rs) | One row per agent turn request |
| `AgentResult` (`turn_id`, `attempt_id`, `command_id`, `session_key`, `conversation_id`, `agent_id`, `status`, `content`, `content_type`, `error`, `tool_calls_count`, `execution_duration_ms`, `secondary_command_attempts`, `command_results`, `trace_id`), `AgentResultStatus`, `WriteOutcome` | [`crates/choruz-writer/src/models.rs`](../../crates/choruz-writer/src/models.rs) | Dispatch to writer over a bounded `mpsc::channel(256)` |
| `OutboxProcessResult` (`reply`, `processed_count`, `command_results`) | [`services/choruz-pipeline/src/outbox_handler.rs`](../../services/choruz-pipeline/src/outbox_handler.rs) | Result of draining one workspace's `.choruz-outbox/new/` |
| `BootstrapOutcome`, `PreservedEditedEntry` | [`services/choruz-pipeline/src/instructions.rs`](../../services/choruz-pipeline/src/instructions.rs) | Per-turn instruction refresh report |
| `EventEnvelope` (`event_id`, `event_type`, `aggregate_type`, `aggregate_id`, `payload`, `trace_id`, `org_id`, `timestamp`, `schema_version` = `CURRENT_SCHEMA_VERSION`) | [`crates/choruz-events/src/envelope.rs`](../../crates/choruz-events/src/envelope.rs) | Wire schema for pipeline events |

`agent_commands.status` is one of `pending`, `leased`, `started`, `heartbeating`, `succeeded`, `committed`, `retry_scheduled`, `dead_letter` (`CommandStatus::as_str`). `session_registry.session_key` is `{agent_id}:{conversation_id}` and carries the `epoch` that fences stale attempts.

Prompts carried in `agent_commands.prompt` start with one of three markers: `[choruz-incoming] ...` built by `build_prompt` in [`router.rs`](../../crates/choruz-router/src/router.rs), `[choruz-batch] You have N pending messages ...` built by `build_batched_prompt` in [`dispatch.rs`](../../services/choruz-pipeline/src/dispatch.rs) when one agent has several pending commands, and `[choruz-cron] job:<name> schedule:<value> | <message>` built in [`cron_scheduler.rs`](../../services/choruz-pipeline/src/cron_scheduler.rs). The envelope format is specified in [agent-protocol](agent-protocol.md).

## Entry points

Process: `target/release/choruz-pipeline` (started by [`infra/host/dev.sh`](../../infra/host/dev.sh) through `pnpm dev:all`, restarted by [`infra/host/pipeline_watchdog.sh`](../../infra/host/pipeline_watchdog.sh) when `/readyz` fails). `main.rs` reads `RUST_LOG`, dispatches `rebootstrap` before boot, then `PipelineConfig::from_env` and `validate`, exiting with code 2 on an invalid config.

| Env var | Default | Effect |
|---|---|---|
| `CHORUZ_DATABASE_URL` or `CHORUZ_PG_*` via `common::PgConfig` | local | PostgreSQL connection |
| `CHORUZ_PIPELINE_CDC_POLL_MS`, `CHORUZ_PIPELINE_CDC_BATCH` | 500, 100 | CDC poll cadence and claim size (claim lease 30s) |
| `CHORUZ_PIPELINE_DISPATCH_MS`, `CHORUZ_PIPELINE_DISPATCH_BATCH` | 1000, 50 | Dispatch fallback tick and `find_pending_commands` limit |
| `CHORUZ_PIPELINE_LEASE_CHECK_MS`, `CHORUZ_PIPELINE_LEASE_TIMEOUT_SECS` | 10000, 60 | Lease monitor cadence and heartbeat timeout (must be at least 45s, three heartbeats) |
| `CHORUZ_PIPELINE_RETRY_CHECK_MS`, `CHORUZ_PIPELINE_RETRY_BATCH` | 5000, 20 | Retry scheduler cadence and batch |
| `CHORUZ_PIPELINE_EXECUTOR_TIMEOUT_SECS` | 1800 | Hard kill timeout for one headless CLI turn |
| `CHORUZ_PIPELINE_SANDBOX_DIR` | `/tmp/choruz-sandboxes` | `SandboxManager` base dir; WAL lives under `<dir>/_wal` |
| `CHORUZ_CLAUDE_CLI_PATH`, `CHORUZ_CODEX_CLI_PATH`, `CHORUZ_PI_CLI_PATH`, `CHORUZ_GROK_CLI_PATH`, `CHORUZ_OPENCODE_CLI_PATH` | binary names | Headless CLI executables per `driver_type` |
| `CHORUZ_PIPELINE_METRICS_HOST`, `CHORUZ_PIPELINE_METRICS_PORT` | `127.0.0.1`, 3020 | Bind address of the fanout/health HTTP server |
| `CHORUZ_PIPELINE_NODE_ID` | `pipeline-local-1` | Executor node id written to claims and leases |
| `CHORUZ_API_BASE_URL` | `http://127.0.0.1:3000` | API Gateway base used by outbox commands (`share_file`, `create_group`, channel tasks, provisioning) |

How a message becomes a reply:

1. A write to `conversation_events` also inserts an `event_outbox` row; the `trg_outbox_notify` trigger fires `choruz_outbox`. `CdcPoller` claims unpublished rows with `claim_unpublished_entries` (`FOR UPDATE SKIP LOCKED`, `claim_deadline`) and forwards `OutboxRow`s.
2. `run_router_loop` parses the event, `route_event` lists agent members via `PgMemberProvider`, evaluates `evaluate_trigger` (`AutoMode::AllMessages` always triggers, `MentionedOnly` matches `@name`, principal id, `mention_aliases` or `@all`, `Manual` never triggers), writes `mailbox_visibility` and `route_decisions`, and for each `Triggered` member writes an `InsertCommand` with `max_attempts = DEFAULT_MAX_ATTEMPTS` through `PgDecisionSink::write_command` (which upserts `session_registry`). The row is `pending` and `trg_command_notify` fires `choruz_commands`; the outbox row is then `mark_published`.
3. `run_dispatch_loop` wakes on `LISTEN choruz_commands` or the dispatch tick, calls `find_pending_commands`, groups rows by `agent_id`, builds one prompt per agent, and takes the whole group with `assign_batch_leases` (one transaction; the primary command moves to `started`, secondaries stay `leased`). A heartbeat task keeps every `session_registry` row alive every `DISPATCH_HEARTBEAT_INTERVAL_SECS` (15s) and flips the primary to `heartbeating`.
4. `execute_command` calls `ExecutorContext::spawn_headless_session`: it resolves the agent's active `agent_runtime_bindings` row, requires `workspace_path` to exist, calls `ensure_claude_md(work_dir, driver_type)`, stages `metadata.attachments` into `<workspace>/.choruz-inbox/`, then spawns the CLI selected by `driver_type` with `HeadlessDriver::args` (Claude `--print --output-format stream-json`, Codex `exec --json`, Pi `--mode json`, Grok `-p --output-format streaming-json`, OpenCode `run --format json`) plus `CHORUZ_WORKSPACE`, `CHORUZ_SEND`, `CHORUZ_OUTBOX_DIR`, `DISABLE_AUTOUPDATER`, `PI_SKIP_VERSION_CHECK`, `CLAUDE_CODE_ENABLE_TASKS` and, for isolated harness accounts, the env returned by `harness_account_env`. `webhook_agent` bindings skip the spawn and only drain the outbox.
5. After the process exits the executor records a new `external_session_id` when the CLI reports one, drains `<workspace>/.choruz-outbox/new/` with `process_outbox_commands_with_stats`, and recovers exact outbox files the CLI wrote from a non-bound workdir (`extract_external_outbox_files`). The reply text of the `AgentResult` is only what outbox commands return; raw CLI stdout is never used as a reply.
6. The dispatch task marks the batch `succeeded` (`mark_command_succeeded_for_attempt`) or, on failure, `retry_scheduled` with `next_retry_at` when `is_auto_retriable_error` and attempts remain, otherwise `dead_letter` via `dead_letter_command_for_attempt`. The `AgentResult` is sent to the writer channel.
7. `run_writer_loop` calls `commit_result`: it skips non-succeeded results, stale attempts (`attempt_is_current`), empty content (group sends were already inserted by `send_to_group`) and already-committed `turn_id`s, then inserts a `conversation_events` row with `event_type = "reply"` and `metadata {command_id, attempt_id, tool_calls_count, execution_duration_ms, trace_id}`, and marks every command in the batch `committed` (`mark_command_committed_for_attempt` releases the session).
8. `FanoutGateway::run_fanout_loop` (2s) reads `conversation_events` through `PgEventSource` and pushes to `/ws/fanout` subscribers; the product dashboard uses the gateway's `/v1/ws/sync` instead (see [sync-feed](sync-feed.md)).

Cron: `run_cron_scheduler` ticks every 30s, claims due `agent_cron_job` rows (`enabled`, `next_run_at <= NOW()`, `running_at` null or older than `CRON_STALE_CLAIM_AFTER` = 10 minutes) with `FOR UPDATE SKIP LOCKED`, inserts a `conversation_events` row from sender `choruz-cron` when `delivery_mode = 'announce'`, and inserts the `agent_commands` row directly. Jobs are created by the gateway routes `GET/POST /v1/agents/{agent_id}/cron` and `PATCH/DELETE /v1/agents/{agent_id}/cron/{job_id}` or by the `set_cron` outbox command.

Outbox watcher: `run_outbox_watcher_loop` ticks every 2s over `RuntimeStore::list_active_bindings` (state not `disabled` or `paused`), drains only bindings whose `driver_type` passes `drains_via_watcher` (terminal drivers and `webhook_agent`) and whose agent has no command in `leased`/`started`/`heartbeating`, and publishes any DM reply text with `publish_watcher_reply`.

CLI subcommand: `choruz-pipeline rebootstrap --workspace <path>` or `choruz-pipeline rebootstrap --principal <agent-principal-id>` (`-w`, `-p`/`--agent`, `-h`/`--help`) calls `force_rewrite_bootstrap`, backs up the previous file to `<name>.<ext>.bak.choruz-rebootstrap`, prints a JSON report with `version`, and exits 0; `--principal` resolves `agent_runtime_bindings.workspace_path` and `driver_type` through `CHORUZ_DATABASE_URL`.

HTTP on the metrics port: `GET /healthz` (`common::HostServiceStatus` for `choruz-pipeline`), `GET /readyz` (200 `ready` when both `EventStore::health_check` and `PgSessionStore::health_check` pass, else 503 `not_ready`), `GET /metrics` (`meta.rs`; the process-wide registry in `crates/choruz-common/src/metrics.rs` as Prometheus text, so a metric registered once with `common::metrics::register_counter` and its siblings from any crate the pipeline links appears without a handler change; `metrics_serves_the_shared_registry_as_prometheus_text`), `GET /ws/fanout` (`WsParams`: `user_id`, `client_id`; unknown query fields are rejected).

## Invariants

| Invariant | Pinned by |
|---|---|
| At most one in-flight batch per agent: `find_pending_commands` excludes agents with a `leased`, `started`, `heartbeating` or `retry_scheduled` row, orders candidates by per-agent `ROW_NUMBER()` so idle agents get a fair first slot, and coalesces an agent's FIFO backlog into one batch | `test_find_pending_commands_gives_idle_agents_a_fair_first_slot`, `test_find_pending_commands_coalesces_per_agent` in [`crates/choruz-session/tests/integration.rs`](../../crates/choruz-session/tests/integration.rs); index `idx_agent_commands_pending_fair` (V022) |
| A batch lease is atomic: if any member is not `pending` the transaction rolls back and no member runs | `assign_batch_leases_rolls_back_when_any_member_is_not_pending`, `assign_batch_leases_same_session_uses_one_epoch` |
| Stale attempts never overwrite a reassigned command; every status write is fenced by `attempt_id`/`epoch` (`SessionError::StaleAttempt`, `EpochMismatch`) | `stale_attempt_cannot_overwrite_reassigned_command_or_heartbeat`, `expired_batch_members_share_one_epoch_fence_and_all_retry`; writer `attempt_is_current` |
| `lease_timeout_secs` is at least three heartbeat intervals | `PipelineConfig::validate`; `lease_timeout_requires_heartbeat_safety_margin` in [`config.rs`](../../services/choruz-pipeline/src/config.rs) |
| A `turn_id` is committed at most once (`conversation_events.turn_id` lookup plus unique-conflict handling) | `duplicate_turn_is_deduped`, `duplicate_group_turn_is_deduped` in [`pipeline_test.rs`](../../services/choruz-pipeline/src/pipeline_test.rs) |
| Only transient errors retry (`kind=timeout`, `killed`, `network`, `rate_limited`, `resume_failure`, `spawn_failure`, `process_failed`); the final allowed attempt dead-letters instead of rescheduling; backoff is `2^attempt` seconds capped at `MAX_BACKOFF_SECS` = 300 with `DEFAULT_MAX_ATTEMPTS` = 3 | `retries_only_transient_executor_failures`, `final_retriable_attempt_is_dead_lettered_without_rescheduling` in [`dispatch.rs`](../../services/choruz-pipeline/src/dispatch.rs); tests in [`retry.rs`](../../crates/choruz-session/src/retry.rs) |
| A command row per (`message_id`, `agent_id`) is inserted once even under concurrent routers | `insert_command_is_idempotent_by_message_and_agent_under_race` |
| Failed results are never written as replies | `failed_result_not_committed` in `pipeline_test.rs`; `commit_result` returns `SkippedNotSucceeded` |
| Dispatch wakes on `choruz_commands` only | `command_listener_wakes_for_choruz_channel_not_legacy_channel` in [`pg_notify.rs`](../../services/choruz-pipeline/src/pg_notify.rs) (runs when `CHORUZ_LISTENER_TEST_DATABASE_URL` is set) |
| Every headless turn refreshes the workspace instruction file before spawning | `ensure_claude_md` call in `spawn_headless_session`; tests in [`instructions.rs`](../../services/choruz-pipeline/src/instructions.rs) |

## Failure modes

| Failure | Behaviour | Operator signal |
|---|---|---|
| Database unreachable at boot | `run_pipeline` logs and returns before spawning tasks | process exits; `/readyz` absent |
| Database unreachable at runtime | `/readyz` returns 503 `not_ready`; loops log and continue | watchdog restarts on `/readyz` failure |
| Any core task exits | `tokio::select!` in `run_pipeline` fires, `ExecutorContext::shutdown_all` runs, process exits | `... task exited` error log |
| CLI hangs | `tokio::time::timeout(executor_timeout)` with `kill_on_drop(true)`; error `[kind=timeout]` is retriable | `cli_hard_timeout` log event |
| CLI binary missing | `classify_cli_start_error` maps `NotFound` to `driver_unavailable`, which is not retriable, so the command dead-letters | `executor_command_failed` with `error_kind`; `dead_letters` row |
| Executor node dies mid-turn | `run_lease_monitor` finds `session_registry` rows past `lease_timeout_secs`; `handle_lease_expiry` bumps the epoch and moves the command to `retry_scheduled` or `dead_letter` | `lease expired` warn log |
| Command pending longer than 24h | `dead_letter_stale_pending_commands(86400, ...)` in the retry scheduler | `dead-lettered stale pending commands` warn |
| Outbox row unparsable or orphaned | `dead_letter_outbox_entry` after `OUTBOX_DEAD_LETTER_AFTER_ATTEMPTS` = 5 attempts, or immediately when the event no longer exists | `dead_letters` row |
| Crash while a Maildir file is `.processing` | `claim_outbox_file` re-claims files older than `PROCESSING_STALE_AFTER` = 60s with a `.retry-<id>.processing` name | none beyond logs |
| `LISTEN` connection drops | reconnect after 2s; the dispatch tick and CDC poll cover the gap | `dispatch LISTEN reconnecting` warn |
| WAL has incomplete turns after a crash | `recover_from_wal` at boot marks them failed | `running WAL crash recovery` log |
| Agent has no active binding or its workspace is missing on disk | turn fails with a non-retriable message | `executor_command_failed` |
| Instruction file is not a recognised managed template | preserved, `tracing::warn`, sidecar `<workspace>/.choruz-bootstrap-warning.json` | run `choruz-pipeline rebootstrap` |

Per-agent state is exposed to operators by `GET /v1/conversations/{conversation_id}/runtime-status` on the API Gateway, which reads `PgSessionStore::list_runtime_status_for_agents` (`busy`, `queued`, `idle`, plus `active_command`, `queued_count`, `last_error`).

## Tests

- [`services/choruz-pipeline/src/pipeline_test.rs`](../../services/choruz-pipeline/src/pipeline_test.rs): in-memory route-then-write scenarios (`route_then_write_end_to_end`, `route_at_all_generates_multiple_commands`, selective-mention cases).
- Unit tests inside [`dispatch.rs`](../../services/choruz-pipeline/src/dispatch.rs), [`executor.rs`](../../services/choruz-pipeline/src/executor.rs), [`outbox_handler.rs`](../../services/choruz-pipeline/src/outbox_handler.rs), [`outbox_watcher.rs`](../../services/choruz-pipeline/src/outbox_watcher.rs), [`cron_scheduler.rs`](../../services/choruz-pipeline/src/cron_scheduler.rs), [`instructions.rs`](../../services/choruz-pipeline/src/instructions.rs), [`config.rs`](../../services/choruz-pipeline/src/config.rs), [`meta.rs`](../../services/choruz-pipeline/src/meta.rs), [`pg_notify.rs`](../../services/choruz-pipeline/src/pg_notify.rs) and the `TestDatabase` cases in [`pg_member_provider.rs`](../../services/choruz-pipeline/src/pg_member_provider.rs); run with `cargo test -p choruz-pipeline`.
- [`crates/choruz-session/tests/integration.rs`](../../crates/choruz-session/tests/integration.rs): leases, epochs, fair dispatch, retry and dead-letter lifecycle against PostgreSQL (`CHORUZ_TEST_DATABASE_URL`).
- Crate-local tests in [`crates/choruz-router/src/router.rs`](../../crates/choruz-router/src/router.rs), [`policy.rs`](../../crates/choruz-router/src/policy.rs), [`workflow.rs`](../../crates/choruz-router/src/workflow.rs), [`crates/choruz-writer/src/writer.rs`](../../crates/choruz-writer/src/writer.rs), [`crates/choruz-fanout/src/gateway.rs`](../../crates/choruz-fanout/src/gateway.rs), [`ws.rs`](../../crates/choruz-fanout/src/ws.rs), [`cursor.rs`](../../crates/choruz-fanout/src/cursor.rs), [`crates/choruz-executor/src/wal.rs`](../../crates/choruz-executor/src/wal.rs), [`sandbox.rs`](../../crates/choruz-executor/src/sandbox.rs), [`crates/choruz-events/src/topics.rs`](../../crates/choruz-events/src/topics.rs), [`crates/choruz-store/src/cdc_poller.rs`](../../crates/choruz-store/src/cdc_poller.rs), [`event_outbox.rs`](../../crates/choruz-store/src/event_outbox.rs).
- Web end to end: [`apps/web/tests/e2e/outbox.spec.ts`](../../apps/web/tests/e2e/outbox.spec.ts) (`Outbox / agent message pipeline`), [`apps/web/tests/e2e/outbox-reply.spec.ts`](../../apps/web/tests/e2e/outbox-reply.spec.ts), [`apps/web/tests/e2e/team-collaboration.spec.ts`](../../apps/web/tests/e2e/team-collaboration.spec.ts).
- Real-driver smoke: [`infra/host/smoke/real-harness-platform-smoke.ts`](../../infra/host/smoke/real-harness-platform-smoke.ts) with [`docs/testing/real-harness-platform-smoke.md`](../testing/real-harness-platform-smoke.md).

## Related

- [agent-protocol](agent-protocol.md): the envelope the router builds and the commands the outbox handler executes.
- [choruz-agent-runtime](agent-runtime.md): `agent_runtime_bindings`, driver types and the terminal path that bypasses this pipeline.
- [store](store.md), [sync-feed](sync-feed.md), [channel-tasks](channel-tasks.md), [threads](threads.md), [choruz-api-gateway](api-gateway.md).
- [architecture.md §5.2](../architecture.md) narrates the same flow across subsystems.
- Agent Notes: [Per-turn roster injection](../../.agents/notes/implemented/architecture/2026-08-18-per-turn-roster-injection.md), [Versioned bootstrap refresh](../../.agents/notes/implemented/feature/2026-08-18-versioned-bootstrap-refresh.md), [Modular monolith](../../.agents/notes/implemented/architecture/2026-08-18-modular-monolith.md).
