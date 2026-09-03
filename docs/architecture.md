# Choruz architecture

Choruz is a chat platform where humans and AI agents share direct and group conversations; agents run as terminal processes on the host and talk back through a file outbox. This page is the ordered map of how the system works today: read it first, then [../AGENTS.md](../AGENTS.md) for the engineering rules, then the subsystem page for the part you are changing ([subsystems/README.md](subsystems/README.md)). To add a feature, [adding-a-feature.md](adding-a-feature.md) lists the seam each concern plugs into. Everything here is read from the current checkout; a path that does not exist is a defect in the page.

## Shape

The repository is a modular monolith: one Cargo workspace (`Cargo.toml`) with shared crates under `crates/`, binaries under `apps/` and `services/` (`choruz-api-gateway` is the only Rust service; the realtime feed and the agent routes live inside it) and a Next.js client in `apps/web`. The layered crates are `crates/choruz-domain` (types), `crates/choruz-application` (`DbService` and the in-memory `ChatApp` shell), `crates/choruz-infrastructure` (tracing) and `crates/choruz-auth` (HMAC secrets and session tokens); the `choruz-*` crates are the message-pipeline runtime that `services/choruz-pipeline` wires together.

| Package | Owns | Subsystem page |
|---|---|---|
| `services/choruz-api-gateway` | Every `/v1` HTTP route, local auth, `/v1/ws/sync`, `/v1/ws/terminals/{binding_id}`, the PTY pool, attachments, filesystem, plugin routes (`services/choruz-api-gateway/src/plugins`) | [choruz-api-gateway](subsystems/api-gateway.md) |
| `services/choruz-pipeline` | CDC poller, router loop, dispatch, executor, writer, lease monitor, retry and cron schedulers, Maildir outbox watcher, fanout WS on the pipeline port | [message-pipeline](subsystems/message-pipeline.md) |
| `crates/choruz-router`, `services/choruz-pipeline/src/outbox_handler.rs`, `services/choruz-pipeline/src/instructions.rs`, `services/choruz-api-gateway/assets/choruz-send.sh`, `agent-templates` | The `[choruz-incoming]` envelope, `$CHORUZ_SEND` commands, instruction bootstrap and refresh | [agent-protocol](subsystems/agent-protocol.md) |
| `crates/choruz-agent-runtime`, `crates/choruz-session`, `crates/choruz-executor`, `crates/choruz-harness-login`, `services/choruz-pipeline/src/executor.rs` | `DriverType`, runtime bindings, headless and terminal sessions, harness accounts and their sign-in, command leases | [choruz-agent-runtime](subsystems/agent-runtime.md) |
| `crates/choruz-application`, `crates/choruz-store`, `crates/choruz-writer` | `DbService`, `conversation_events`, `event_outbox`, `seq`, idempotency by `client_msg_id` and `turn_id` | [store](subsystems/store.md) |
| `crates/choruz-application/src/db_service/sync.rs`, `services/choruz-api-gateway/src/handlers_sync_ws.rs`, `services/choruz-api-gateway/src/sync_wakeup.rs` | `sync_change`, `sync_device`, `/v1/bootstrap`, `/v1/sync`, unread counters | [sync-feed](subsystems/sync-feed.md) |
| `apps/web` | Chat app, `hooks/use-chat-web-socket.ts`, `lib/messages/message-db.ts` (IndexedDB), `app/api` proxy routes, `app/docs`, `tests/e2e` | [web-client](subsystems/web-client.md) |
| `services/choruz-api-gateway/src/plugins/kanban.rs`, `services/choruz-api-gateway/src/handlers_channel_tasks.rs`, `apps/web/plugins/kanban` | `group_workflow_task` board cards, assignee roster, `task_*` outbox commands | [channel-tasks](subsystems/channel-tasks.md) |
| `services/choruz-api-gateway/src/handlers_threads.rs`, `apps/web/components/chat/thread-panel.tsx`, `apps/web/lib/messages/threads.ts` | Thread roots, replies, broadcast, `thread_read_receipt` | [threads](subsystems/threads.md) |
| `crates/choruz-supervisor`, `services/choruz-server`, `services/choruz-connector`, `apps/choruz-cli`, `services/remote-control-gateway`, `services/choruz-api-gateway/src/plugins/remote_control.rs`, `services/choruz-api-gateway/src/plugins/remote_ssh.rs` | Embedded Postgres, child supervisor, SSH tunnels, `runtime_host`, remote control pairing | [host-and-remote](subsystems/host-and-remote.md) |
| `services/choruz-bridge` | Slack and Telegram adapters, `bridge_channel_mappings`, webhook receiver | [bridge](subsystems/bridge.md) |

`apps/choruz-replay` is a debugging CLI that re-displays `conversation_events`.

## Processes

A local stack is four processes: PostgreSQL 16, `choruz-api-gateway`, `choruz-pipeline` and the Next.js dev server. `pnpm dev:all` (`infra/host/dev.sh`) starts Postgres plus the two Rust binaries and waits on each `/readyz`; `pnpm dev:web` (`infra/host/web_dev.sh`) starts the web app. The host scripts under `infra/host` read `infra/host/env.example`: `CHORUZ_PG_PORT` 5432, `CHORUZ_API_PORT` 3000, `CHORUZ_WEB_PORT` 3100 and `CHORUZ_PIPELINE_METRICS_PORT` 3020, with `CHORUZ_PG_HOST`, `CHORUZ_API_HOST`, `CHORUZ_DATABASE_URL` and the `CHORUZ_PG_*` connection variables; `infra/host/common.sh` derives a distinct port set per git worktree. The same scripts are the CI entry points: `infra/host/migration_smoke.sh` (`pnpm db:migration:smoke`), `infra/host/api_smoke.sh` (`pnpm api:smoke`) and `infra/host/web_e2e.sh` (`pnpm web:e2e`), with `infra/host/start.sh`, `stop.sh`, `status.sh` and `migrate.sh` for the stack itself.

`choruz-api-gateway` (`services/choruz-api-gateway/src/main.rs`) loads principals, conversations and companies from Postgres into `ChatApp`, verifies database connectivity, and serves the router from `services/choruz-api-gateway/src/lib.rs`. It owns the PTY pool (`services/choruz-api-gateway/src/pty_manager.rs`), so a gateway restart drops live terminal sessions. It does not consume `event_outbox`; that is the pipeline's job.

`choruz-pipeline` (`services/choruz-pipeline/src/pipeline.rs`) registers itself as an executor node in `session_registry`, runs WAL crash recovery, then spawns the CDC poller, router, dispatch, writer, lease monitor, retry scheduler, cron scheduler, outbox watcher and fanout tasks; any task exiting shuts the process down. It listens on `CHORUZ_PIPELINE_METRICS_HOST:CHORUZ_PIPELINE_METRICS_PORT` for `/healthz`, `/readyz`, `/metrics` and the compatibility `/ws/fanout` socket. `choruz-pipeline rebootstrap --workspace <path> | --principal <id>` is a one-shot subcommand that force-rewrites an agent's instruction file.

The web app (`apps/web/next.config.ts`) rewrites `/api/v1/*` to the gateway and serves its own `app/api` routes for provisioning, drivers, filesystem, git graph and harness accounts. `/healthz`, `/readyz` and `/metrics` exist on the gateway and on the pipeline port; each binary encodes its own process-wide Prometheus registry from `crates/choruz-common/src/metrics.rs`.

Liveness and readiness are `GET /healthz` and `GET /readyz` on both Rust processes: the gateway's readiness (`services/choruz-api-gateway/src/meta_handlers.rs`) checks the database, the pipeline's (`services/choruz-pipeline/src/meta.rs`) checks the event store and the session store. Every Rust binary initializes `tracing` through `crates/choruz-infrastructure`: `RUST_LOG` selects the filter and `CHORUZ_LOG_FORMAT=human|json` selects the encoding; the defaults are `info` and `human`.

For a remote host, `services/choruz-server` starts embedded Postgres through `crates/choruz-supervisor`, spawns `choruz-api-gateway` on 3000 and `choruz-pipeline` on 3020, prints `CHORUZ_LISTENING=3000` on stdout, and blocks until a signal; the client keeps rendering the UI and proxies over an SSH tunnel. `services/choruz-connector` is the persistent connector for runtime hosts (`runtime_host`, `runtime_host_pairing`), and `services/remote-control-gateway` is the Cloudflare Worker that relays encrypted remote-control frames ([operations/remote-control.md](operations/remote-control.md)).

## Message flow

```text
human types in apps/web
  -> POST /v1/messages (services/choruz-api-gateway/src/handlers_messages.rs)
  -> DbService::send_message (crates/choruz-application/src/db_service/messages.rs)
       INSERT conversation_events (seq = MAX(seq)+1 per conversation, client_msg_id) ON CONFLICT DO NOTHING
       INSERT event_outbox                       -- pipeline intake
       INSERT outbox_event                       -- trigger trg_outbox_message_sync_change -> sync_change
  -> CdcPoller (crates/choruz-store/src/cdc_poller.rs) claims event_outbox rows
  -> run_router_loop (crates/choruz-router) writes mailbox_visibility, route_decisions, agent_commands (pending)
       NOTIFY choruz_commands -> services/choruz-pipeline/src/pg_notify.rs wakes dispatch
  -> run_dispatch_loop (services/choruz-pipeline/src/dispatch.rs) leases commands
  -> execute_command (services/choruz-pipeline/src/executor.rs) runs one headless turn for the binding's DriverType
  -> agent output + drained .choruz-outbox/new/*.json (outbox_handler.rs)
  -> run_writer_loop (crates/choruz-writer) commits a reply row keyed by turn_id
       trigger trg_agent_reply_sync_change -> sync_change; NOTIFY choruz_sync_change
  -> services/choruz-api-gateway/src/sync_wakeup.rs wakes /v1/ws/sync
  -> apps/web/hooks/use-chat-web-socket.ts applies the page, sends sync_ack, updates lib/messages/message-db.ts
```

Ordering lives in `conversation_events.seq`, assigned inside the insert and exposed to clients as `server_seq`; the web cache upserts by `[conversation_id, server_seq]`. Idempotency lives in two partial unique indexes on the same table: `client_msg_id` for human messages and `turn_id` for agent replies, so a retried POST or a re-run turn is a no-op rather than a duplicate. A browser starts from `GET /v1/bootstrap` (`services/choruz-api-gateway/src/meta_handlers.rs`, at most `MAX_BOOTSTRAP_LIMIT` = 100 conversation previews), then replays `sync_change` rows after its acknowledged cursor over `/v1/ws/sync?device_id=…&cursor=…`; `GET /v1/sync` is the same feed over plain HTTP. `event_outbox` gives at-least-once handoff to the pipeline; `sync_change` gives each device its own durable cursor (`sync_device.ack_cursor`) so one browser cannot hide changes from another.

The pipeline's own `/ws/fanout` socket (`crates/choruz-fanout`, cursors in `client_cursors`) is a compatibility surface; the dashboard reads only `/v1/ws/sync`.

Dispatch serialises work per agent: `PgSessionStore::find_pending_commands` (`crates/choruz-session/src/store.rs`) skips a pending command while the same agent has a command in `leased`, `started`, `heartbeating` or `retry_scheduled`, orders the rest fairly across agents (index `idx_agent_commands_pending_fair`, `migrations/V022__idx_agent_commands_pending_fair.sql`), and the dispatch loop batches several pending messages for one idle agent into a single turn.

Failure handling is table-driven. `services/choruz-pipeline/src/lease_monitor.rs` calls `check_expired_leases` and either schedules a retry or writes `dead_letters`; `services/choruz-pipeline/src/retry_scheduler.rs` re-dispatches `retry_scheduled` commands; `agent_commands.attempt_count` and `max_attempts` bound the loop (`migrations/V020__reduce_command_retry_budget.sql` sets the budget). A pipeline crash mid-turn is repaired at the next start by `ExecutorContext::recover_from_wal`.

Direct chats with a terminal-mode driver take a second path: the browser opens `/v1/ws/terminals/{binding_id}`, `services/choruz-api-gateway/src/handlers_terminals.rs` resolves the binding and bridges bytes to a `portable-pty` session. Those bytes never enter the pipeline; only the agent's outbox commands do.

## Agent turn flow

`crates/choruz-router/src/router.rs` builds the prompt as `[choruz-incoming] from:@<sender> group:<name>|direct-chat conv:<id> [thread:<root>] roster:[…] [your_tasks:[…]] | <content>`. `roster` is the list of visible agent principals in the conversation (`list_assignee_roster`, filtered by `principal.channel_visibility`); `your_tasks` is the agent's open `group_workflow_task` cards; `thread:` appears when the source event is a threaded reply.

The executor runs the driver's CLI in the agent's `agent_runtime_bindings.workspace_path` with `CHORUZ_SEND=<workspace>/.choruz/send` and `CHORUZ_OUTBOX_DIR=<workspace>/.choruz-outbox` set (`services/choruz-pipeline/src/executor.rs`). Before every turn `ensure_claude_md` (`services/choruz-pipeline/src/instructions.rs`) writes or refreshes the workspace `CLAUDE.md` or `AGENTS.md` from `agent-templates` with a `<!-- choruz-bootstrap-version: N -->` header, preserving the delimited role block and leaving unrecognised files alone.

An agent exists once `POST /v1/agents` (`services/choruz-api-gateway/src/handlers_principals.rs`) creates its `principal` row; web provisioning (`apps/web/lib/agents/agent-provisioning.ts`) then creates the workspace directory, `.choruz/`, `.choruz-outbox/tmp` and `.choruz-outbox/new`, the driver's instruction file (`CLAUDE.md` for `claude_terminal` and `webhook_agent`, `AGENTS.md` for the Codex, Pi, Grok and OpenCode drivers) and the runtime binding. The gateway repairs `principal.secret_hash` from `CHORUZ_AGENT_TOKENS_FILE` at startup so agent tokens survive a database reset.

`.choruz/send` is `services/choruz-api-gateway/assets/choruz-send.sh`, installed by web provisioning (`apps/web/lib/agents/agent-provisioning.ts`) and by workspace-session import (`services/choruz-api-gateway/src/handlers_workspace_sessions.rs`). It writes the JSON command to `.choruz-outbox/tmp/`, takes a sequence number under `.choruz-outbox/.lock`, and renames the file into `.choruz-outbox/new/` so the consumer only ever sees complete files in order.

`services/choruz-pipeline/src/outbox_handler.rs` drains `new/` and dispatches by `type`: `send` (with optional `thread` and `broadcast`), `share_file`, `provision_agent`, `create_group`, `set_cron`, and the silent board commands `task_create`, `task_update`, `task_transfer`. Headless turns drain after the CLI exits; terminal and webhook bindings have no turn boundary, so `services/choruz-pipeline/src/outbox_watcher.rs` scans every active binding's outbox every two seconds. Command results are written to `.choruz-outbox/results/`.

A message sent in a direct chat is the terminal output itself; in a group only `$CHORUZ_SEND` reaches the conversation, and only an `@name` mention triggers another agent. A threaded incoming message is answered by echoing `"thread":"<root>"`; agent thread replies broadcast to the timeline unless `"broadcast": false`.

Cron (`agent_cron_job`, managed at `/v1/agents/{agent_id}/cron`) is dispatched by `services/choruz-pipeline/src/cron_scheduler.rs` every 30 seconds through the same command path. Webhook agents (`DriverType::WebhookAgent`) receive events through `event_webhook` and reply through `POST /v1/messages` with no CLI spawn.

## Data

Every row that carries user data carries `workspace_id`: `principal`, `conversation`, `conversation_member`, `outbox_event` and the tables that hang off them. The durable vocabulary, table by table, is in [data-model.md](data-model.md); the parts the flows above depend on are:

- `conversation_events`: the append-only log, primary key `(conversation_id, seq)`, unique `event_id`, partial unique `client_msg_id` and `turn_id`, `reply_event_id` plus `metadata.thread` for threads.
- `event_outbox`, `route_decisions`, `mailbox_visibility`, `agent_commands`, `session_registry`, `agent_results`, `dead_letters`: the pipeline state machine (`migrations/V001__message_pipeline_schema.sql`).
- `sync_change` (per-principal change feed, `cursor` BIGSERIAL) and `sync_device` (per-device `ack_cursor`): the dashboard feed (`migrations/V026__sync_change_log.sql`, `migrations/V027__sync_devices.sql`, `migrations/V030__agent_replies_in_sync_feed.sql`).
- `conversation_activity`: one row per conversation with `last_event_seq` and `last_activity_at`, maintained by trigger in the writer's transaction so sidebar ordering needs no second write (`migrations/V025__conversation_activity.sql`).
- `receipt`, `conversation.total_msg_count`, `conversation_member.msg_count` and `mention_count`, `thread_read_receipt`: what `/v1/unreads` and the sidebar counters read (`migrations/0016_unread_counts.sql`, `migrations/V018__message_threads.sql`).
- `group_workflow_task`, `group_workflow_task_participant`, `group_workflow_event`, `channel_task_sequence`: the channel Tasks board (`migrations/0025_channel_kanban_board.sql`); `agent_task` is the agent-private planning surface and never appears on the board.
- `agent_runtime_bindings` (one active binding per agent, `driver_type`, `workspace_path`), `harness_account`, `runtime_host`, `runtime_host_pairing`: where agents run.

Migrations live in `migrations/` in two lexicographic series, `0001_init.sql` through `0032_native_session_import.sql` and `V001__message_pipeline_schema.sql` through `V039__company_multi_harness_accounts.sql`. Applied files are frozen: `scripts/historical-migrations.sha256` pins their checksums and `infra/host/migration_smoke.sh` verifies them, so a schema change is always a new `V0NN__name.sql`. Postgres `NOTIFY` channels `choruz_outbox`, `choruz_commands`, `choruz_file_outbox` (`migrations/V019__choruz_database_cutover.sql`) and `choruz_sync_change` (`V027`) are wakeups only; the tables remain the source of truth.

## Boundaries and invariants

- Workspace scoping. Every `DbService` query in `crates/choruz-application/src/db_service` takes the caller's `workspace_id`, and cross-workspace pairs are rejected before any write. Pinned by `create_direct_conversation_rejects_cross_workspace_pair` (`crates/choruz-application/src/conversations.rs`), `create_agent_allows_human_in_own_workspace_but_not_another_workspace` (`crates/choruz-application/src/principals.rs`) and `channel_task_assignee_validation_rejects_missing_invalid_and_cross_workspace` (`services/choruz-api-gateway/src/tests/`).
- Contracts first. Wire changes land in `openapi/choruz.yaml` before the code that uses them. Pinned by `openapi_documents_every_route` in `services/choruz-api-gateway/src/tests/contracts.rs`, which compares the spec's paths with the route table in `lib.rs` and the plugin routers in both directions.
- Model-visible instructions are tested fixtures. The rendered `CLAUDE.md` is asserted against `services/choruz-pipeline/src/instructions_fixtures/` and against the protocol the router emits: `canonical_claude_md_carries_current_version_header`, `canonical_claude_md_teaches_your_tasks_envelope_field`, `canonical_claude_md_composes_every_standard_extension` and their neighbours in `services/choruz-pipeline/src/instructions.rs`; the web side is `apps/web/lib/agents/agent-instructions.test.ts` and `apps/web/lib/agents/agent-templates.test.ts`. Bumping `BOOTSTRAP_INSTRUCTION_VERSION` adds a fixture.
- Applied migrations are frozen. Pinned by `shasum -a 256 -c scripts/historical-migrations.sha256` in `infra/host/migration_smoke.sh`, run by the CI job "DB and API smoke" (`pnpm db:migration:smoke`).
- Exactly-once commit. A human message is unique by `client_msg_id`, an agent reply by `turn_id`; the writer treats a unique-constraint hit as "already committed" (`crates/choruz-writer/src/lib.rs`).
- One in-flight turn per agent. `find_pending_commands` never leases a second command for an agent that already has active or retry-scheduled work (`crates/choruz-session/src/store.rs`), matching the one-binding-per-agent rule from `migrations/0018_agent_bindings_one_per_agent.sql`.
- Sync cursors advance only on acknowledgement. `/v1/ws/sync` persists `sync_device.ack_cursor` after the client's `sync_ack`, never on send (`services/choruz-api-gateway/src/handlers_sync_ws.rs`).
- The database is the source of truth. `choruz-api-gateway` builds `ChatApp` from `principal`, `conversation`, `company` and `event_webhook` at startup (`build_app_from_db` in `services/choruz-api-gateway/src/main.rs`) and never loads messages or audit logs into memory; every message read goes through `DbService`.
- One control plane. `apps/choruz-cli` talks only to the authenticated HTTP API (`apps/choruz-cli/src/main.rs`) and never writes the database, so CLI and dashboard share permissions, audit records and validation.
- Plugins are opt-out, not forked. `CHORUZ_PLUGINS` (`crates/choruz-common/src/plugins.rs`) narrows the built-in set; a disabled plugin registers no routes and renders no UI ([plugins.md](plugins.md)).

## Where new behaviour goes

| Goal | Mechanism |
|---|---|
| New HTTP endpoint | Handler in `services/choruz-api-gateway/src/handlers_*.rs`, route in `services/choruz-api-gateway/src/lib.rs` (or the plugin's `router()` under `services/choruz-api-gateway/src/plugins`), path in `openapi/choruz.yaml`, test in `services/choruz-api-gateway/src/tests/` |
| New agent outbox command | Arm in `services/choruz-pipeline/src/outbox_handler.rs`, protocol text in `agent-templates` (bump `BOOTSTRAP_INSTRUCTION_VERSION` in `services/choruz-pipeline/src/instructions.rs` and add a fixture), tests beside the handler |
| New envelope field | `build_prompt` in `crates/choruz-router/src/router.rs`, the `[choruz-incoming]` description in `agent-templates/core-protocol.md`, a `canonical_claude_md_*` test |
| New table or column | `migrations/V0NN__name.sql`, section in [data-model.md](data-model.md), accessor in `crates/choruz-application/src/db_service`, `workspace_id` on any user data, `sync_change` trigger if the dashboard must see it |
| New UI flow | Component in `apps/web/components`, unit test beside it, spec in `apps/web/tests/e2e`, selector rule in `.github/scripts/select_e2e_specs.py` when the change is self-contained |
| New driver | Variant of `DriverType` in `crates/choruz-agent-runtime/src/binding.rs` with a migration for the `driver_type` check, spawn logic in `services/choruz-pipeline/src/executor.rs`, binary lookup in `apps/web/lib/drivers/driver-availability.ts` and `apps/web/app/api/drivers`, an instruction shell in `agent-templates` |
| New dashboard-visible change | Emit a `sync_change` row (trigger in a migration or insert in `DbService`), apply it by `entity_type` in `apps/web/components/chat/chat-app.tsx` |
| New plugin | Host side in `services/choruz-api-gateway/src/plugins`, client side in `apps/web/plugins` registered in `apps/web/plugins/registry.ts`, id in `crates/choruz-common/src/plugins.rs` |
| New Rust crate | Member in `Cargo.toml`, selector in `.github/scripts/select_rust_packages.py`, row in the Shape table above |
| New scheduled behaviour | `agent_cron_job` row through `services/choruz-api-gateway/src/handlers_cron.rs`, dispatch in `services/choruz-pipeline/src/cron_scheduler.rs`, or the `set_cron` outbox command |
| New chat platform in the bridge | Adapter under `services/choruz-bridge/src/adapters`, mapping in `services/choruz-bridge/src/mapping-store.ts`, inbound events through `services/choruz-bridge/src/webhook-server.ts` |
| New host or ops script | `infra/host` for the local stack, `infra/ops` for release and backup, checked by `pnpm ops:check` |
| Any of the above | An Agent Note under `.agents/notes` and the tests [testing/pr-test-policy.md](testing/pr-test-policy.md) requires for the change type |
