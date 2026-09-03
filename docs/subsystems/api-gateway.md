# API gateway

The API gateway is the Axum process that serves every `/v1` route, the terminal and dashboard-sync WebSockets, and the health endpoints; it authenticates each request from a signed session token or an agent secret, validates the payload, and delegates all durable work to the store. Source: [`services/choruz-api-gateway`](../../services/choruz-api-gateway/src/lib.rs) with token primitives in [`crates/choruz-auth`](../../crates/choruz-auth/src/lib.rs).

## Owns

- [`services/choruz-api-gateway/src/lib.rs`](../../services/choruz-api-gateway/src/lib.rs): `router_with_runtime` builds the `Router`, registers every route, constructs `ApiState`, and wraps the app in `meta_handlers::request_logging_middleware`; [`main.rs`](../../services/choruz-api-gateway/src/main.rs) loads `Config::from_env`, verifies database connectivity, and serves with graceful shutdown.
- [`config.rs`](../../services/choruz-api-gateway/src/config.rs) (`Config`), [`local_auth.rs`](../../services/choruz-api-gateway/src/local_auth.rs) (`LocalAuthConfig::authenticate`), [`auth.rs`](../../services/choruz-api-gateway/src/auth.rs) (`ApiError`, `authenticated_principal`, `require_actor`, `require_self`, `require_human_operator`, `redact_sensitive_text`), [`state.rs`](../../services/choruz-api-gateway/src/state.rs) (`ApiState`, `PtyPool`).
- [`crates/choruz-auth/src/lib.rs`](../../crates/choruz-auth/src/lib.rs): `SESSION_COOKIE_NAME` (`choruz_session`), `SessionClaims`, `issue_session_token`, `verify_session_token`, `issue_secret`, `hash_secret`, `verify_secret`, `local_user_principal_id`.
- Handler modules, one per surface: `handlers_principals.rs` (login, signup, agents), `handlers_conversations.rs`, `handlers_messages.rs`, `handlers_threads.rs`, `handlers_events.rs`, `handlers_companies.rs`, `handlers_runtime.rs`, `handlers_runtime_status.rs`, `handlers_runtime_hosts.rs`, `handlers_terminals.rs`, `handlers_sync_ws.rs`, `handlers_cron.rs`, `handlers_tasks.rs`, `handlers_channel_tasks.rs`, `handlers_filesystem.rs`, `handlers_ssh.rs`, `handlers_remote_control.rs`, `handlers_workspace_sessions.rs`, [`meta_handlers.rs`](../../services/choruz-api-gateway/src/meta_handlers.rs), [`ingress.rs`](../../services/choruz-api-gateway/src/ingress.rs).
- [`plugins/mod.rs`](../../services/choruz-api-gateway/src/plugins/mod.rs): `registrations()` lists the six built-in Host plugins; a plugin's router is merged only when `common::plugins::plugin_enabled(id)` is true for the `CHORUZ_PLUGINS` allowlist. Routers exist for `kanban`, `remote-ssh`, and `remote-control`; `pixel-world`, `workspace-git`, and `agent-skills` are manifest-only.
- Background tasks owned by the process: `keepalive::spawn_keepalive_task` (60s PTY keepalive), `sync_wakeup::SyncWakeupHub` (PostgreSQL `LISTEN`), `remote_control_bridge::spawn`, and the in-process `PtyPool` of terminal sessions.

Route groups registered in `lib.rs` (paths are literal; `{}` marks Axum path parameters):

| Group | Routes |
|---|---|
| Health | `GET /healthz`, `GET /readyz`, `GET /metrics`, `GET /v1/status` |
| Auth and identity | `POST /v1/auth/local/login`, `GET /v1/auth/local/bootstrap`, `POST /v1/auth/local/signup`, `GET /v1/me` |
| Dashboard state | `GET /v1/bootstrap`, `GET /v1/sync`, `GET /v1/ws/sync`, `GET /v1/console`, `GET /v1/unreads` |
| Principals and agents | `POST /v1/agents`, `POST /v1/agents/batch-disable`, `POST /v1/agents/{agent_id}/rotate-secret`, `POST /v1/principals/{principal_id}/disable`, `PATCH /v1/principals/{principal_id}/workspace` |
| Conversations | `GET /v1/conversations`, `POST /v1/conversations/direct`, `POST /v1/groups`, `PATCH /v1/groups/{conversation_id}`, `POST /v1/groups/{conversation_id}/members`, `DELETE /v1/groups/{conversation_id}/members/{principal_id}`, `PUT|DELETE /v1/conversations/{conversation_id}/pin`, `PUT|DELETE .../archive`, `PUT|DELETE .../hide`, `PATCH .../workspace`, `POST .../view` |
| Messages | `POST /v1/messages`, `GET /v1/messages/search`, `GET /v1/conversations/{conversation_id}/messages`, `GET .../message-page`, `GET .../messages/{message_id}`, `GET .../threads/{thread_root_id}`, `POST .../threads/{thread_root_id}/view`, `POST /v2/ingest` |
| Attachments | `POST /v1/attachments` (34 MB body cap), `GET|DELETE /v1/attachments/{attachment_id}` |
| Events and webhooks | `GET /v1/principals/{principal_id}/events`, `POST .../events/ack`, `POST .../event-webhook`, `POST /v1/webhooks/flush`, `POST /v1/telemetry` |
| Runtime | `GET|POST /v1/runtime/bindings`, `GET /v1/runtime/bindings/{binding_id}`, `POST .../rebind`, `GET|PUT /v1/runtime/policies/{conversation_id}`, `GET /v1/conversations/{conversation_id}/runtime-status`, `POST /v1/companies/{company_id}/reset-sessions` |
| Terminals | `GET /v1/ws/terminals/{binding_id}`, `POST /v1/terminals/{binding_id}/ensure`, `POST /v1/terminals/{binding_id}/input` |
| Agent tasks and cron | `GET /v1/agents/{agent_id}/tasks`, `GET|POST /v1/agents/{agent_id}/cron`, `PATCH|DELETE /v1/agents/{agent_id}/cron/{job_id}` |
| Companies and audit | `GET|POST /v1/companies`, `GET|PATCH|DELETE /v1/companies/{company_id}`, `POST .../archive`, `POST .../unarchive`, `GET|POST .../members`, `DELETE .../members/{member_id}`, `GET /v1/audit-logs`, `GET /v1/export/conversations/{conversation_id}` |
| Filesystem | `GET /v1/filesystem/list`, `GET .../stat`, `GET .../home`, `GET .../read`, `POST .../write` |
| Plugin `kanban` | `GET|POST /v1/conversations/{conversation_id}/tasks`, `POST .../tasks/from-message`, `GET|PATCH /v1/tasks/{task_id}` ([`plugins/kanban.rs`](../../services/choruz-api-gateway/src/plugins/kanban.rs)) |
| Plugin `remote-ssh` | `GET /v1/ssh/hosts`, `POST /v1/ssh/tunnel`, `GET /v1/ssh/tunnels`, `DELETE /v1/ssh/tunnel/{id}`, `POST /v1/ssh/connect-choruz` ([`plugins/remote_ssh.rs`](../../services/choruz-api-gateway/src/plugins/remote_ssh.rs)) |
| Plugin `remote-control` | `/v1/remote-control/*` pairing and device routes, `/v1/companies/{company_id}/runtime-host-pairings`, `/v1/runtime-hosts/{host_id}/*` (heartbeat, commands, harness accounts, harness-account logins), `PUT /v1/runtime/bindings/{binding_id}/host`, `POST /v1/workspace-sessions/scan`, `POST /v1/workspace-sessions/import` ([`plugins/remote_control.rs`](../../services/choruz-api-gateway/src/plugins/remote_control.rs)) |

## Data

- `domain::Principal` ([`crates/choruz-domain/src/lib.rs`](../../crates/choruz-domain/src/lib.rs)) is the authenticated identity: `id`, `workspace_id`, `principal_type` (`human` or `agent`), `name`, `scopes`, `secret_hash`, `disabled`, `deleted_at`, `channel_visibility`, `user_id`.
- Session token: `SessionClaims { principal_id, workspace_id, display_name, expires_at_epoch_s }` serialised as `<base64url(json)>.<base64url(hmac-sha256)>`, signed with `CHORUZ_SESSION_SECRET`, verified in constant time, and delivered either as `Authorization: Bearer <token>` or the `choruz_session` cookie (`HttpOnly; SameSite=Lax; Path=/`).
- Agent secret: `issue_secret()` returns `agt_<uuidv7>`; only `hash_secret` (SHA-256 hex) is stored in `principal.secret_hash`, and `DbService::authenticate_agent_secret` matches a presented bearer value against every active agent hash with `verify_secret`.
- Local operator: `LocalAuthConfig` derives the operator principal id with `local_user_principal_id(workspace, display_name)` (SHA-256 of `workspace:lowercased-name`, prefixed `local-user-`) so repeated logins converge on one row.
- Error shape: every handler returns `ApiError(common::AppError)`, rendered as `{"error":{"status":<u16>,"detail":"<redacted text>"}}` with `Unauthorized` → 401, `NotFound` → 404, `Conflict` → 409, `Validation` → 400, `Forbidden` → 403, `RateLimited { retry_after_ms }` → 429, `Internal` → 500.
- Request bodies are the `serde` structs in [`crates/choruz-application/src/types.rs`](../../crates/choruz-application/src/types.rs) (`SendMessageRequest`, `CreateAgentRequest`, `CreateGroupRequest`, `ListMessagesQuery`, `MessagePageQuery`, ...) and `ingress::IngestRequest` / `IngestResponse { message_id, seq, deduplicated }`.
- Harness accounts: `harness_account` ([`V035__harness_accounts.sql`](../../migrations/V035__harness_accounts.sql)) stores per-company, per-`driver_type` (`claude_terminal` or `codex_terminal`) login metadata with `profile_kind`, `status`, `models_json`, and `usage_json`; `harness_account_login` ([`V036__remote_harness_account_logins.sql`](../../migrations/V036__remote_harness_account_logins.sql), [`V037__local_harness_account_logins.sql`](../../migrations/V037__local_harness_account_logins.sql)) tracks one sign-in state machine per account, whether a runtime host's connector or the gateway's own device (`runtime_host_id` NULL) runs it. The `validate_runtime_binding_harness_account` trigger rejects a runtime binding whose `config_json.harness_account_id` is not an active account for the same company, driver, host, and model.
- `HostPluginManifest { id, version, host_capabilities, client_capabilities }` is returned by `/v1/bootstrap` and `/v1/console` so the web client can pair Host and Client plugin halves.

Environment read by the gateway ([`config.rs`](../../services/choruz-api-gateway/src/config.rs), [`crates/choruz-common/src/lib.rs`](../../crates/choruz-common/src/lib.rs), [`crates/choruz-common/src/plugins.rs`](../../crates/choruz-common/src/plugins.rs)): `CHORUZ_API_HOST` (default `127.0.0.1`), `CHORUZ_API_PORT` (default `3000`), `CHORUZ_ENV`, `CHORUZ_SESSION_SECRET`, `CHORUZ_OPERATOR_PASSWORD`, `CHORUZ_OPERATOR_WORKSPACE` (default `ws-local`), `CHORUZ_OPERATOR_USER` (default `operator`), `CHORUZ_SESSION_TTL_HOURS` (default `87600`), `CHORUZ_ATTACHMENT_DIR`, `CHORUZ_AGENT_TOKENS_FILE`, `CHORUZ_DATABASE_URL` or `CHORUZ_PG_HOST` / `CHORUZ_PG_PORT` / `CHORUZ_PG_USER` / `CHORUZ_PG_DB` / `CHORUZ_PG_PASSWORD`, `CHORUZ_PLUGINS`, `CHORUZ_LOG_FORMAT`.

## Entry points

- Every request passes `request_logging_middleware`, which counts it, records latency buckets, propagates an incoming `x-trace-id` into the tracing span, and stamps `x-request-id` on the response.
- Authentication runs inside each handler, not as a global layer: `authenticated_principal` calls `LocalAuthConfig::authenticate`, which takes the bearer token first, then the `choruz_session` cookie, verifies it as a session token and loads the principal with `DbService::get_principal`, and otherwise treats the value as an agent secret. `require_actor` and `require_self` bind the session to the `actor_id` / `principal_id` in the request; `require_human_operator` rejects agents from control-plane routes.
- Sessions are minted by `handlers_principals::local_login` (operator credentials from the environment, or a signed-up human matched by `find_human_by_username` and `verify_secret`), `local_signup` (`create_human_user`), and `local_bootstrap` (loopback only: peer address, `Host` header, and absence of `Forwarded` / `x-forwarded-for` / `x-real-ip` are all checked, then a `303` to `http://127.0.0.1:<return_port>/dashboard` sets the cookie).
- Write handlers call `DbService::check_rate_limit` (600 requests per principal per minute, in-memory per instance) before touching the store; `send_message` then delegates to `DbService::send_message`, announces the row to in-process webhook consumers with `ChatApp::inject_message_with_event`, and runs `flush_webhooks_all`.
- Read handlers for conversation content share one membership gate, `handlers_threads::require_conversation_read_access`, so `list_messages`, `list_message_page`, `get_thread`, and `view_thread` cannot drift.
- Validation lives in the handler or in `DbService`: for example `list_message_page` rejects `before_seq` together with `after_seq` and clamps `limit` to 1..100, `send_message` rejects blank content or a blank `idempotency_key`, `local_bootstrap` rejects `return_port = 0`, and `register_sync_device` rejects a `device_id` longer than 128 characters.
- Work leaves the gateway through `DbService` writes (see [store.md](store.md)), the per-principal `sync_change` feed (see [sync-feed.md](sync-feed.md)), `outbox_event` rows for webhooks, and PTY bytes on `/v1/ws/terminals/{binding_id}`.

## Invariants

- The sender of a message is always the authenticated principal: `send_message` enforces `require_actor` on `actor_id`, and `POST /v2/ingest` rejects a `sender_id` field in the body. Pinned by `ingest_request_rejects_sender_id`, `agent_privacy_surfaces_are_scoped_to_authorized_workspace_context` and `company_workspace_authorization_guards_hold` in [`tests.rs`](../../services/choruz-api-gateway/src/tests/).
- Only the `choruz_session` cookie is honoured; a legacy cookie name is neither accepted nor selected (`legacy_session_cookie_is_not_accepted_or_selected` in `local_auth.rs`, `session_cookie_uses_only_the_choruz_identity` in `crates/choruz-auth`).
- A session token verifies only with the issuing secret and only before `expires_at_epoch_s` (`session_tokens_round_trip_and_expire` in `crates/choruz-auth`).
- `GET /v1/auth/local/bootstrap` never mints a session for a proxied or remote browser (`local_bootstrap_only_issues_sessions_to_loopback_browsers`), and concurrent operator logins resolve to a single principal row (`concurrent_local_operator_logins_converge_on_one_principal`).
- Error details never leak credentials: `sanitize_app_error` runs `redact_sensitive_text` over `Bearer `, `token=`, `secret=`, and `password=` markers (`api_error_responses_redact_secrets`).
- With `CHORUZ_ENV=production`, the process refuses to start on the default `CHORUZ_SESSION_SECRET` or `CHORUZ_OPERATOR_PASSWORD` (`Config::validate_production`); an explicitly empty secret is rejected in every environment (`explicit_empty_secret_is_rejected`).
- Plugin routes are registered only for enabled plugins, and the registration order matches `common::plugins::BUILTIN_PLUGIN_IDS` (`builtin_manifests_have_matching_host_and_client_contracts` in `plugins/mod.rs`).
- Every response carries an `x-request-id`. `/metrics` (`meta_handlers.rs`) refreshes the `choruz_principals_total`, `choruz_conversations_total`, `choruz_messages_total`, `choruz_audit_logs_total` and `choruz_event_backlog_total` gauges from `ChatApp::metrics_snapshot` and returns the process-wide registry in `crates/choruz-common/src/metrics.rs` as Prometheus text (`text/plain; version=0.0.4`); the request middleware feeds `choruz_http_requests_total` and the `choruz_http_request_duration` histogram (buckets 0.05, 0.2, 1 s). A feature adds a metric by registering it once from a `LazyLock` static with `common::metrics::register_counter` (or `register_gauge`, `register_histogram`, `register_counter_vec`) and incrementing it where the event happens; the endpoint lists nothing by hand (`metrics_endpoint_reports_prometheus_text`).

## Failure modes

- PostgreSQL unreachable at boot: `main.rs` prints `FATAL: cannot connect to database` and exits with status 1. While running, `/readyz` returns `503` with a `common::HostServiceStatus` body (`status: "not_ready"`, `service: "choruz-api-gateway"`, `protocol_version`, `database: false`) whenever `RuntimeStore::health_check` fails; `/healthz` stays `200`.
- Sync listener not ready: `/v1/ws/sync` waits up to 5 seconds on `SyncWakeupHub::wait_ready` and then answers `500` with detail `dashboard sync unavailable: ...`.
- Missing or invalid credentials: `401` with `missing credentials`, `invalid local credentials`, or `invalid agent secret`; an agent calling a human control-plane route gets `403 only signed-in people can manage workspace agents`.
- Rate limit exceeded: `429` with `retry_after_ms: 1000`; the window is per gateway instance and resets on restart.
- Internal errors are logged at `tracing::error!` with the redacted message and returned as `500`; operators correlate through `x-request-id` and the `trace_id` field on the request span.
- Insecure defaults in development are logged as warnings (`CHORUZ_SESSION_SECRET not set, using insecure default`).
- Gateway restart drops every live PTY in `PtyPool`; terminal clients must reconnect and `ensure_terminal` again.
- If `choruz-pipeline` is not running, `event_outbox` rows written by `send_message` and `ingest_message` stay unpublished but are not lost.

## Tests

- [`services/choruz-api-gateway/src/tests/`](../../services/choruz-api-gateway/src/tests/): PostgreSQL-backed integration tests that boot the router with `router_with_runtime`; auth and contract cases include `local_bootstrap_only_issues_sessions_to_loopback_browsers`, `concurrent_local_operator_logins_converge_on_one_principal`, `api_error_responses_redact_secrets`, `company_workspace_authorization_guards_hold`, `runtime_bindings_list_detail_and_redact_errors`, `harness_account_binding_trigger_rejects_unverified_models`, and `runtime_host_pairing_is_single_use_and_host_token_is_revocable`.
- Inline unit tests in [`crates/choruz-auth/src/lib.rs`](../../crates/choruz-auth/src/lib.rs), [`local_auth.rs`](../../services/choruz-api-gateway/src/local_auth.rs), [`config.rs`](../../services/choruz-api-gateway/src/config.rs), [`plugins/mod.rs`](../../services/choruz-api-gateway/src/plugins/mod.rs), [`ingress.rs`](../../services/choruz-api-gateway/src/ingress.rs), and [`handlers_messages.rs`](../../services/choruz-api-gateway/src/handlers_messages.rs).
- Browser end-to-end: [`apps/web/tests/e2e/auth.spec.ts`](../../apps/web/tests/e2e/auth.spec.ts), [`api-routes.spec.ts`](../../apps/web/tests/e2e/api-routes.spec.ts), [`server.spec.ts`](../../apps/web/tests/e2e/server.spec.ts), [`plugins.spec.ts`](../../apps/web/tests/e2e/plugins.spec.ts), and the backend sweep [`apps/web/tests/e2e/sweep-be-api.spec.ts`](../../apps/web/tests/e2e/sweep-be-api.spec.ts).
- Host smoke: [`infra/host/api_smoke.sh`](../../infra/host/api_smoke.sh) and [`infra/host/smoke.sh`](../../infra/host/smoke.sh).
- Contract: [`openapi/choruz.yaml`](../../openapi/choruz.yaml) lists every registered route; `openapi_documents_every_route` in [`src/tests/contracts.rs`](../../services/choruz-api-gateway/src/tests/contracts.rs) compares it with `lib.rs` and the plugin routers in both directions. Route registration in `lib.rs` remains the implementation source.

## Related

- [store.md](store.md) for `DbService` and the tables every handler writes.
- [sync-feed.md](sync-feed.md) for `/v1/bootstrap`, `/v1/sync`, and `/v1/ws/sync`.
- [agent-runtime.md](agent-runtime.md) and [host-and-remote.md](host-and-remote.md) for terminals, runtime hosts, and harness accounts.
- [channel-tasks.md](channel-tasks.md) and [threads.md](threads.md) for the `kanban` plugin routes and the thread read side.
- [web-client.md](web-client.md) for the Next.js caller.
- [`openapi/choruz.yaml`](../../openapi/choruz.yaml) for the endpoint reference.
- Agent Notes: [OpenAPI as the one external contract](../../.agents/notes/implemented/architecture/2026-09-03-openapi-single-contract.md).
