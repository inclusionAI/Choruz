# Adding a feature

A feature is a vertical slice: a domain type and its queries, a route or command, a table, an authorization gate, a log line or metric, tests, a rollout switch, a compatibility promise and a page that describes it. Every one of those horizontal concerns has exactly one extension point in this repository, listed below in the order a contributor meets them. You write the business logic and plug it into each seam; you never re-implement a seam (a second connection pool, a private auth check, a handler that formats its own metrics, a spec that no selector rule knows about).

Read this page before opening a `feature`, `api`, `database`, `security` or `auth` pull request ([testing/pr-test-policy.md](testing/pr-test-policy.md) defines the types) and record the seams you touched in the pull request template's "Seams touched" section. Each section names the extension point, one shipped example, and what breaks or what CI catches when the seam is skipped. The channel Tasks board ([subsystems/channel-tasks.md](subsystems/channel-tasks.md)) is the recurring example because it crosses every seam.

## Domain

Row types live in [`crates/choruz-domain/src/lib.rs`](../crates/choruz-domain/src/lib.rs) (`Principal`, `Conversation`, `AuditLog`, ...). Reads and writes live in `DbService`, one file per area under [`crates/choruz-application/src/db_service/`](../crates/choruz-application/src/db_service/) (`audit.rs`, `companies.rs`, `conversations.rs`, `events.rs`, `group_workflow_tasks.rs`, `messages.rs`, `principals.rs`, `sync.rs`), each declared as a module in [`mod.rs`](../crates/choruz-application/src/db_service/mod.rs); row-to-type converters such as `row_to_principal` sit in [`helpers.rs`](../crates/choruz-application/src/db_service/helpers.rs). A new area is a new `<area>.rs` plus its `mod` line. `DbService::new(store: EventStore)` wraps the one connection pool from [`crates/choruz-store/src/pool.rs`](../crates/choruz-store/src/pool.rs); the gateway holds it as `ApiState.db` ([`services/choruz-api-gateway/src/state.rs`](../services/choruz-api-gateway/src/state.rs)), so a handler calls `state.db.<method>` and never opens its own pool. Every command in `crates/choruz-application` checks `workspace_id` ([AGENTS.md](../AGENTS.md#conventions)).

Example: channel tasks are [`group_workflow_tasks.rs`](../crates/choruz-application/src/db_service/group_workflow_tasks.rs) (`create_channel_task`, `patch_channel_task`, `channel_task_observability_counters`), and every handler in `handlers_channel_tasks.rs` goes through those methods.

Skipped: SQL inside a handler or a second pool bypasses the workspace checks and the counters that `DbService` methods carry. No gate catches it mechanically; the [code review skill](../.agents/skills/choruz-code-review/SKILL.md) blocks it.

## Interface

Every `/v1` route is registered explicitly with `.route(...)` inside `router_with_runtime` in [`services/choruz-api-gateway/src/lib.rs`](../services/choruz-api-gateway/src/lib.rs); handlers live in `services/choruz-api-gateway/src/handlers_<area>.rs`. A plugin-owned route is registered in its descriptor under [`services/choruz-api-gateway/src/plugins/`](../services/choruz-api-gateway/src/plugins/) and merged by `plugins::router()` only while the plugin is enabled (see [Rollout](#rollout)). The wire contract is written first: [`openapi/choruz.yaml`](../openapi/choruz.yaml) lists every route the gateway registers ([AGENTS.md](../AGENTS.md#conventions), "Contracts first"), and `openapi_documents_every_route` in [`services/choruz-api-gateway/src/tests/contracts.rs`](../services/choruz-api-gateway/src/tests/contracts.rs) fails on a path present on one side only. A Next.js route under [`apps/web/app/api/`](../apps/web/app/api/) starts with `requireAuth` from [`apps/web/lib/api/api-auth.ts`](../apps/web/lib/api/api-auth.ts), which verifies the session cookie against the gateway before the route trusts any claim.

Example: [`plugins/kanban.rs`](../services/choruz-api-gateway/src/plugins/kanban.rs) routes `/v1/conversations/{conversation_id}/tasks` and `/v1/tasks/{task_id}` to [`handlers_channel_tasks.rs`](../services/choruz-api-gateway/src/handlers_channel_tasks.rs), and `openapi/choruz.yaml` documents both paths.

Skipped: a route that is not in `router_with_runtime` or a plugin descriptor does not exist. A route missing from `openapi/choruz.yaml` is a red Rust tests job, not a review finding.

## Persistence

Schema changes are new files only: `migrations/V0NN__name.sql` with the next number (the tree ends at `V039__company_multi_harness_accounts.sql`). Applied files are pinned by checksum in [`scripts/historical-migrations.sha256`](../scripts/historical-migrations.sha256), which [`infra/host/migration_smoke.sh`](../infra/host/migration_smoke.sh) verifies with `shasum -a 256 -c` before applying the chain to a fresh database; CI runs it as the `DB and API smoke` job on any change under `migrations/**`, `crates/**` or `services/**`. A new table carrying user data carries `workspace_id` ([AGENTS.md](../AGENTS.md#conventions)) and a row in [data-model.md](data-model.md).

Example: `audit_log` (`0001_init.sql`) carries `workspace_id TEXT NOT NULL`; `group_workflow_task` (`0024_hybrid_agent_routing.sql`) is scoped through `conversation_id`, and its assignee queries in `group_workflow_tasks.rs` join back to `conversation.workspace_id`.

Skipped: an edit to a pinned file fails the checksum step of the migration smoke; a table without `workspace_id` passes CI and is blocked in review.

## Authorization

Authentication runs inside each handler, not as a global layer, through one of these helpers:

| Helper | Defined in | Grants the request when |
|---|---|---|
| `require_actor` | [`auth.rs`](../services/choruz-api-gateway/src/auth.rs) | the session principal is the `actor_id` the request names |
| `require_self` | [`auth.rs`](../services/choruz-api-gateway/src/auth.rs) | the session principal is the `principal_id` in the path |
| `require_human_operator` | [`auth.rs`](../services/choruz-api-gateway/src/auth.rs) | the session principal is a human; agents are refused on control-plane routes |
| `require_company_access` | [`handlers_companies.rs`](../services/choruz-api-gateway/src/handlers_companies.rs) | the company is not deleted and the principal is one of its members |
| `require_conversation_read_access` | [`handlers_threads.rs`](../services/choruz-api-gateway/src/handlers_threads.rs) | the conversation is live and the principal holds an active membership reachable through a non-deleted company |
| `require_agent_workspace_access` | [`handlers_cron.rs`](../services/choruz-api-gateway/src/handlers_cron.rs) | a human operator's workspace or companies include the agent; returns the agent's `workspace_id` |
| `require_host` | [`handlers_runtime_hosts.rs`](../services/choruz-api-gateway/src/handlers_runtime_hosts.rs) | the `x-choruz-host-token` header hashes to a non-revoked `runtime_host` row |

A handler that needs a rule none of these express adds a helper beside them and reuses it from every route with the same rule. A mutation worth an audit trail calls `DbService::record_audit` ([`audit.rs`](../crates/choruz-application/src/db_service/audit.rs)), which inserts `audit_log(workspace_id, actor_id, action, target_type, target_id, metadata)`. Every command in `crates/choruz-application` checks `workspace_id` ([AGENTS.md](../AGENTS.md#conventions)); the rationale is [workspace-scoped isolation](../.agents/notes/implemented/architecture/2026-08-18-workspace-scoped-isolation.md).

Example: `list_messages` in `handlers_messages.rs` and the thread handlers share `require_conversation_read_access`, so the two read surfaces cannot drift; `create_company` in [`db_service/companies.rs`](../crates/choruz-application/src/db_service/companies.rs) calls `record_audit`.

Skipped: a route without a helper is reachable by any bearer token. The gateway tests pin the existing gates (`company_workspace_authorization_guards_hold`, `channel_task_read_apis_enforce_membership_and_safe_projection`) but cannot know about a new route; the reviewer walks the "Seams touched" section against the diff.

## Observability

Logging is `tracing`: every Rust binary calls `init_tracing` from [`crates/choruz-infrastructure/src/lib.rs`](../crates/choruz-infrastructure/src/lib.rs), which reads `RUST_LOG` and `CHORUZ_LOG_FORMAT=human|json`. Every gateway route inherits `request_logging_middleware` from [`services/choruz-api-gateway/src/meta_handlers.rs`](../services/choruz-api-gateway/src/meta_handlers.rs), layered on the whole router in `router_with_runtime`: it assigns a request id, propagates `x-trace-id`, records method, path and latency, and increments `request_count`. A feature adds `tracing::info!`/`warn!` spans in its handler and `DbService` methods; it never installs a subscriber.

Metrics go through one process-wide Prometheus registry in [`crates/choruz-common/src/metrics.rs`](../crates/choruz-common/src/metrics.rs): `register_counter`, `register_counter_vec`, `register_gauge`, `register_histogram` and `text()`. A feature declares its metric once as a static (`static CREATES: LazyLock<IntCounter> = LazyLock::new(|| metrics::register_counter("choruz_<area>_<event>_total", "..."))`) next to the code that increments it, and it appears on `GET /metrics` of whichever binary links the crate. The gateway handler `meta_handlers::metrics` refreshes the five database gauges and returns `text()`; the pipeline's meta server serves the same `text()` on its `/metrics`. Nothing formats metric text by hand.

Web telemetry is [`apps/web/lib/api/choruz-trace.ts`](../apps/web/lib/api/choruz-trace.ts) (`trace.start`, `span.end`, `trace.event`), which posts batches to `/api/v1/telemetry` (gateway `POST /v1/telemetry`, `handlers_events::ingest_telemetry`) after [`telemetry-sanitize.ts`](../apps/web/lib/api/telemetry-sanitize.ts) redacts secrets and byte payloads.

Example: `CHANNEL_TASK_CREATES_TOTAL`, `CHANNEL_TASK_UPDATES_TOTAL` and `CHANNEL_TASK_MUTATION_ERRORS_TOTAL` in [`group_workflow_tasks.rs`](../crates/choruz-application/src/db_service/group_workflow_tasks.rs), pinned by `metrics_endpoint_reports_channel_task_mutation_counters` in [`tests/channel_tasks.rs`](../services/choruz-api-gateway/src/tests/channel_tasks.rs).

Skipped: a feature without a span is invisible in the request log except as a path and a status; `metrics_endpoint_reports_prometheus_text` in [`tests/observability.rs`](../services/choruz-api-gateway/src/tests/observability.rs) fails when a metric name is emitted twice.

## Testing

Rust unit tests sit next to the code; gateway integration tests are one topic file each under [`services/choruz-api-gateway/src/tests/`](../services/choruz-api-gateway/src/tests/) (`channel_tasks.rs`, `threads.rs`, `sync.rs`, `observability.rs`, ...), sharing the helpers in `tests/mod.rs`. Web unit tests are vitest `*.test.ts` files beside the module ([`choruz-trace.test.ts`](../apps/web/lib/api/choruz-trace.test.ts)); on a pull request CI runs `vitest related` for the changed files. User-visible flows get a Playwright spec in [`apps/web/tests/e2e/`](../apps/web/tests/e2e/), and the `RULES` tuple in [`.github/scripts/select_e2e_specs.py`](../.github/scripts/select_e2e_specs.py) maps the feature's web files to its specs: a new feature area adds a rule and a case in [`.github/scripts/tests/test_select_e2e_specs.py`](../.github/scripts/tests/test_select_e2e_specs.py). [testing/pr-test-policy.md](testing/pr-test-policy.md) maps each pull request type to the tests it must add; [choruz-ci-test-reliability](../.agents/skills/choruz-ci-test-reliability/SKILL.md) owns isolation under parallel workers.

Example: the rule `("components/channel-tasks/**", "components/chat/channel-conversation-tabs.tsx", "lib/channel-task*")` selects `tests/e2e/channel-tasks.spec.ts`.

Skipped: a spec without a rule runs only on `main` after merge or under the `ci-full` label, so a regression reaches `main` first; a web change no rule contains falls back to the P0 set and pays for the whole smoke suite.

## Rollout

The only switch is `CHORUZ_PLUGINS`, a comma-separated allowlist over `BUILTIN_PLUGIN_IDS` in [`crates/choruz-common/src/plugins.rs`](../crates/choruz-common/src/plugins.rs) (`plugin_enabled`, `enabled_plugin_ids`; unset means every built-in on). The gateway merges a plugin's router only when it is enabled and publishes the enabled manifests on `/v1/console`; the web registry [`apps/web/plugins/registry.ts`](../apps/web/plugins/registry.ts) activates a client plugin only when the host manifest has the same id and version, and a plugin-owned Next.js route is guarded by `serverPluginEnabled` ([`apps/web/plugins/server-plugin.ts`](../apps/web/plugins/server-plugin.ts)). [plugins.md](plugins.md) is the procedure for adding a built-in. There is no per-user or per-workspace feature flag and no admin surface for one: a feature is either a core route that is always on or a plugin that a host operator enables per process. The pre-release stance in [AGENTS.md](../AGENTS.md#pre-release-stance-foundation-over-blast-radius) applies: no transition flags, no "legacy" branches.

Example: `kanban` disabled removes the channel-task routes and the Tasks tab together; the rows in `group_workflow_task` stay.

Skipped: a feature that reads an ad-hoc environment variable to gate itself has no manifest, so the client cannot know whether to render it; `builtin_manifests_have_matching_host_and_client_contracts` in [`plugins/mod.rs`](../services/choruz-api-gateway/src/plugins/mod.rs) pins the host and client catalogues against each other.

## Compatibility

Four promises hold across a deploy, and a feature keeps each of them at its own seam. Migrations are frozen (see [Persistence](#persistence)). Wire and configuration contracts change before the code and with the SDKs (see [Interface](#interface)). The sync feed is cursor-compatible, not version-negotiated: a device presenting a cursor ahead of the feed gets `400 sync cursor is ahead of this principal's feed` and the web client resets to the bootstrap cursor with a fresh `device_id` ([subsystems/sync-feed.md](subsystems/sync-feed.md)); a new `sync_change` event type needs a client branch in `handleSyncChanges`, otherwise it triggers a bootstrap refresh. Agent instruction files carry a `choruz-bootstrap-version` header (`BOOTSTRAP_INSTRUCTION_VERSION` in `services/choruz-pipeline/src/instructions.rs`) so the pipeline re-renders an older managed layout and leaves an edited one alone ([subsystems/agent-protocol.md](subsystems/agent-protocol.md)); a change to model-visible text bumps it and updates the fixtures. Plugin manifests are matched by id and version (see [Rollout](#rollout)).

Everything else follows the pre-release rule in [AGENTS.md](../AGENTS.md#pre-release-stance-foundation-over-blast-radius): delete the old path and update every caller in the same change; no shims.

Skipped: the instruction tests in `instructions.rs` catch a stale bootstrap fixture; a wire change that lands without its SDKs is a review finding under "Contracts first".

## Documentation

The subsystem page under [subsystems/](subsystems/README.md) that owns the area is updated in the same pull request (its README fixes the skeleton: Owns, Data, Entry points, Invariants, Failure modes, Tests, Related); a new area gets a new page and a row in that README's table. A new table gets its section in [data-model.md](data-model.md). Every non-trivial change adds or updates an Agent Note under [`.agents/notes/`](../.agents/notes/README.md#when-to-write-one). [architecture.md](architecture.md) changes only when the map changes: a new package, a new process or a new step in the message flow. Prose rules are in [AGENTS.md](AGENTS.md) (this folder) and the [choruz-doc](../.agents/skills/choruz-doc/SKILL.md) skill.

Example: channel tasks own [subsystems/channel-tasks.md](subsystems/channel-tasks.md), a `group_workflow_task` section in `data-model.md`, and a row in the `architecture.md` package table.

Skipped: CI runs `python3 scripts/verify_agent_notes.py` on any change under `.agents/`, so a malformed or missing note fails Static checks; a stale subsystem page passes CI and is caught only by the reviewer's "Docs match the code" check.
