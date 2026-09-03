# Agent runtime

The agent runtime is where an agent actually executes: the `agent_runtime_bindings` row that ties an agent principal to a `driver_type` and a workspace, the terminal (PTY) sessions the API Gateway spawns for direct chats, the headless CLI turns the pipeline spawns for group work, the command leases in `session_registry`, harness accounts (which CLI login a binding uses), hidden agent sessions, and the import of native Claude/Codex/Pi/Grok/OpenCode sessions into Choruz. A reader can use this page to add a driver, to debug why a terminal or headless turn will not start, or to understand binding, account and session state. Source: [`crates/choruz-agent-runtime/`](../../crates/choruz-agent-runtime/), [`crates/choruz-session/`](../../crates/choruz-session/), [`crates/choruz-executor/`](../../crates/choruz-executor/), [`services/choruz-api-gateway/src/handlers_terminals.rs`](../../services/choruz-api-gateway/src/handlers_terminals.rs).

## Owns

| Path | Role |
|---|---|
| [`crates/choruz-agent-runtime/src/binding.rs`](../../crates/choruz-agent-runtime/src/binding.rs) | `DriverType`, `BindingState`, `RuntimeBinding`, `RuntimeStore` (create, get, list, rebind, state and cursor updates, `disable_bindings_by_agent`, `write_terminal_session_anchor`, `begin_codex_terminal_capture`, `sync_session_id_from_disk`, `backfill_session_ids`), `normalize_workspace_path` |
| [`crates/choruz-agent-runtime/src/headless.rs`](../../crates/choruz-agent-runtime/src/headless.rs) | `HeadlessDriver` (`Claude`, `Codex`, `Pi`, `Grok`, `OpenCode`), `from_driver_type`, `args`, `parse_output`, `validate_model`, `configure_command_workspace`, `harness_account_env` |
| [`crates/choruz-agent-runtime/src/policy.rs`](../../crates/choruz-agent-runtime/src/policy.rs) | `ConversationRuntimePolicy`, `AutoMode`, `UntaggedHumanMode`, `RuntimeStore::get_policy` / `upsert_policy` over `conversation_runtime_policies` |
| [`crates/choruz-agent-runtime/src/session_catalog.rs`](../../crates/choruz-agent-runtime/src/session_catalog.rs) | `HarnessKind`, `SessionCatalogScanner::from_env` / `scan`, `NativeSessionSummary`, `SessionScanResult` |
| [`crates/choruz-session/src/store.rs`](../../crates/choruz-session/src/store.rs) | `PgSessionStore`: `session_registry` epochs, `agent_commands` leases (`assign_lease`, `assign_batch_leases`, `release_lease`), `list_runtime_status_for_agents`, runtime-host commands (`claim_runtime_host_command`, `heartbeat_runtime_host_command`, `complete_runtime_host_command`) |
| [`crates/choruz-executor/src/sandbox.rs`](../../crates/choruz-executor/src/sandbox.rs), [`wal.rs`](../../crates/choruz-executor/src/wal.rs), [`adapter.rs`](../../crates/choruz-executor/src/adapter.rs) | `SandboxManager` / `WorkspaceConfig`, `AdapterWal` (turn start/chunk/finished/failed, `find_incomplete_turns`), `CliAdapter` / `CliResponse` |
| [`services/choruz-pipeline/src/executor.rs`](../../services/choruz-pipeline/src/executor.rs) | Headless spawn per `driver_type`, `session_provenance_matches`, resume-failure detection (see [message-pipeline](message-pipeline.md)) |
| [`services/choruz-api-gateway/src/handlers_terminals.rs`](../../services/choruz-api-gateway/src/handlers_terminals.rs), [`state.rs`](../../services/choruz-api-gateway/src/state.rs), [`pty_manager.rs`](../../services/choruz-api-gateway/src/pty_manager.rs) | `websocket_terminal`, `ensure_terminal`, `terminal_input`, `authorize_terminal_binding`, `ensure_pty_session`, `PtySession`, `PtyPool`, `evict_stale_pty_sessions`, `ProcessContainer` |
| [`services/choruz-api-gateway/src/handlers_runtime.rs`](../../services/choruz-api-gateway/src/handlers_runtime.rs), [`handlers_runtime_status.rs`](../../services/choruz-api-gateway/src/handlers_runtime_status.rs) | Binding CRUD, `validate_runtime_host`, `validate_harness_account`, policies, `reset_company_sessions`, `get_conversation_runtime_status` |
| [`services/choruz-api-gateway/src/handlers_runtime_hosts.rs`](../../services/choruz-api-gateway/src/handlers_runtime_hosts.rs) | Harness account registration and verification by a runtime host; runtime-host command claim (routes registered by [`plugins/remote_control.rs`](../../services/choruz-api-gateway/src/plugins/remote_control.rs)) |
| [`services/choruz-api-gateway/src/handlers_harness_logins.rs`](../../services/choruz-api-gateway/src/handlers_harness_logins.rs) | Harness account sign-in: the company-facing start/read/callback routes, the in-process executor for accounts on the gateway's device, the host-facing claim/publish/complete routes the connector uses |
| [`crates/choruz-harness-login/`](../../crates/choruz-harness-login/) | The Claude Code and Codex browser sign-in driver shared by the gateway and `choruz-connector` |
| [`services/choruz-api-gateway/src/handlers_workspace_sessions.rs`](../../services/choruz-api-gateway/src/handlers_workspace_sessions.rs) | `scan_workspace_sessions`, `import_workspace_sessions`, `native_session_import_lock_key`, `ensure_outbox_helper` |
| [`services/choruz-api-gateway/src/handlers_conversations.rs`](../../services/choruz-api-gateway/src/handlers_conversations.rs) | `hide_agent_session`, `restore_hidden_agent_session` |
| [`apps/web/lib/drivers/driver-availability.ts`](../../apps/web/lib/drivers/driver-availability.ts), [`apps/web/app/api/drivers/availability/route.ts`](../../apps/web/app/api/drivers/availability/route.ts), [`models/route.ts`](../../apps/web/app/api/drivers/models/route.ts) | `getDriverAvailability`, `resolveDriverBinary`; `GET /api/drivers/availability` |
| [`apps/web/app/api/harness-accounts/`](../../apps/web/app/api/harness-accounts/) | Web routes for listing, creating, probing, hiding and signing in harness accounts; `POST /api/harness-accounts/default` registers (and, on request, verifies) the login a device already has |

Tables and migrations: `agent_runtime_bindings`, `agent_turn_leases`, `conversation_runtime_policies` ([`0003`](../../migrations/0003_agent_runtime_bridge.sql)); `driver_type` allowlist ([`0004`](../../migrations/0004_runtime_terminal_mode.sql), [`0013`](../../migrations/0013_driver_type_gemini.sql), [`0021`](../../migrations/0021_driver_type_webhook_agent.sql), [`0026`](../../migrations/0026_disable_gemini_driver.sql), [`0027`](../../migrations/0027_finalize_disable_gemini_driver.sql), [`0028`](../../migrations/0028_add_pi_grok_opencode_drivers.sql), [`0029`](../../migrations/0029_finalize_pi_grok_opencode_drivers.sql)); one active binding per agent ([`0018`](../../migrations/0018_agent_bindings_one_per_agent.sql)); hybrid routing columns ([`0024`](../../migrations/0024_hybrid_agent_routing.sql)); `native_session_import` ([`0032`](../../migrations/0032_native_session_import.sql), [`V032`](../../migrations/V032__imported_sessions_use_terminal_ui.sql), [`V033`](../../migrations/V033__imported_sessions_set_harness_binary.sql)); `conversation_hidden` ([`V034`](../../migrations/V034__hidden_agent_sessions.sql)); `harness_account` and the `validate_runtime_binding_harness_account` trigger ([`V035`](../../migrations/V035__harness_accounts.sql)); `harness_account_login` ([`V036`](../../migrations/V036__remote_harness_account_logins.sql), [`V037`](../../migrations/V037__local_harness_account_logins.sql)); `audit_log` rows for binding changes ([`0001`](../../migrations/0001_init.sql)).

## Data

`DriverType` (`as_str`) and where each runs:

| `driver_type` | Terminal (gateway PTY) | Headless (pipeline) | Instruction file |
|---|---|---|---|
| `claude_terminal` | yes | yes (`HeadlessDriver::Claude`) | `CLAUDE.md` |
| `codex_terminal` | yes | yes (`Codex`) | `AGENTS.md` |
| `pi_terminal` | yes | yes (`Pi`) | `AGENTS.md` |
| `grok_terminal` | yes | yes (`Grok`) | `AGENTS.md` |
| `opencode_terminal` | yes | yes (`OpenCode`) | `AGENTS.md` |
| `claude_print`, `codex_exec`, `codex_app_server` | no | yes | `CLAUDE.md` / `AGENTS.md` |
| `acp` | no | no (`from_driver_type` returns `None`) | none |
| `webhook_agent` | no | outbox drain only; events leave through `event_webhook` and `POST /v1/webhooks/flush` | `CLAUDE.md` |

The constraint `agent_runtime_bindings_driver_type_check` (0028/0029) allows exactly `claude_print`, `claude_terminal`, `codex_exec`, `codex_app_server`, `codex_terminal`, `pi_terminal`, `grok_terminal`, `opencode_terminal`, `acp`, `webhook_agent`; `gemini_terminal` is not allowed and 0026 deletes its bindings. `is_terminal_driver` in `handlers_terminals.rs` and `drains_via_watcher` in `outbox_watcher.rs` name the five `*_terminal` drivers.

`RuntimeBinding` ([`binding.rs`](../../crates/choruz-agent-runtime/src/binding.rs)): `id`, `conversation_id`, `agent_principal_id`, `driver_type`, `workspace_path`, `git_worktree_path`, `external_session_id`, `external_thread_id`, `last_event_cursor`, `last_acked_event_cursor`, `last_seen_server_seq`, `state` (`idle`, `running`, `paused`, `disabled`, `error`; transitions guarded by `BindingState::can_transition_to`), `last_error`, `in_flight_turn_id`, `last_trigger_message_id`, `config_json`. Keys read from `config_json`: `model`, `binary_path`, `runtime_host_id`, `harness_account_id`, `harness_account_name`, `harness_account_profile_kind` (`default` or `isolated`), `interaction_mode`, `external_session_mode`, `external_session_driver_type`, `external_session_provenance` (`process_captured` or `workspace_scan_verified`), `external_session_binding_id`, `terminal_session` (`TerminalSessionAnchor`), `terminal_capture` (`CodexTerminalCaptureMetadata`), `native_session_import`, `is_primary`.

`harness_account` rows: `company_id`, optional `runtime_host_id`, `driver_type` in (`claude_terminal`, `codex_terminal`), `name`, `profile_kind`, `account_fingerprint`, `subscription_type`, `status` (`pending`, `active`, `reauth_required`, `error`, `disabled`), `models_json`, `usage_json`, `probed_at`. Each device and harness has one `default` account, the login the device already has; `ensureDefaultHarnessAccount` ([`apps/web/lib/agents/harness-accounts.ts`](../../apps/web/lib/agents/harness-accounts.ts)) registers it the first time the Harness Accounts dialog or agent provisioning needs it, the dialog probes a local one on open, and provisioning binds a Claude Code or Codex agent to it when no account was chosen (`defaultHarnessAccountForLaunch`; an unverified default leaves the binding without an account, so the agent inherits the device's login without quota data). `company.multi_harness_accounts` (V039, off by default) is the switch that lets the dialog add `isolated` sign-ins and Create Agent or Create Group choose among them. Hiding an account (`DELETE`, `disabled_at`) leaves the device's credentials and profile directory in place. Credentials stay in the device-local profile directory; `harness_account_env` resolves an `isolated` profile under `CHORUZ_HARNESS_ACCOUNT_ROOT` (default `$HOME/.choruz/accounts`) and returns the env var the CLI needs. `harness_account_login` rows carry the sign-in state machine (`queued`, `awaiting_browser`, `authorizing`, `verified`, `failed`, `cancelled`, `expired`) with `authorization_url`, `user_code`, `callback_code`, `expires_at`; `runtime_host_id` names the connector that runs the sign-in and is NULL when the API gateway runs it on its own device. A verified Harness identity makes the account active before the independent model and quota refresh finishes. A failed refresh preserves the active status and last complete snapshot, and does not advance `probed_at`.

`native_session_import` maps (`workspace_path`, `driver_type`, `native_session_id`) to one agent principal, one direct conversation and one binding. `NativeSessionSummary` (`harness`, `native_session_id`, `title`, `workspace_path`, `updated_at`, `model`, `branch`, `archived`) is what `SessionCatalogScanner::scan` returns; it reads `$HOME`, `CODEX_HOME` (default `~/.codex`), `PI_CODING_AGENT_SESSION_DIR` (default `~/.pi/agent/sessions`), `GROK_HOME` (default `~/.grok`) and runs `CHORUZ_OPENCODE_BINARY` for OpenCode.

`AgentRuntimeStatus` (`status` = `busy`, `queued` or `idle`, `active_command`, `queued_count`, `last_error`) is computed from `agent_commands` by `list_runtime_status_for_agents`.

Binaries: terminals use `config_json.binary_path` or `default_terminal_binary`, overridden by `CHORUZ_CLAUDE_BINARY`, `CHORUZ_CODEX_BINARY`, `CHORUZ_PI_BINARY`, `CHORUZ_GROK_BINARY`, `CHORUZ_OPENCODE_BINARY` when the configured value is the bare default; headless turns use `CHORUZ_*_CLI_PATH`; `getDriverAvailability` checks the same `CHORUZ_*_BINARY` variables and reports `available` or `unavailable` with `reason`, `setupHint` and `envVar`.

## Entry points

| Route | Handler | Notes |
|---|---|---|
| `GET`/`POST /v1/runtime/bindings`, `GET /v1/runtime/bindings/{binding_id}`, `POST .../rebind` | `handlers_runtime` | Humans only; `create_runtime_binding` returns the existing active binding instead of creating a second; validates `runtime_host_id` and `harness_account_id`; the list, the by-id read and `GET /v1/bootstrap` share one JOIN query (`list_binding_views`), and every view carries `interaction_mode` (`terminal` for a driver the gateway serves over a PTY, else `message`, unless the binding stores its own) |
| `PUT /v1/runtime/bindings/{binding_id}/host` | `handlers_runtime_hosts::assign_binding_host` | remote-control plugin |
| `GET`/`PUT /v1/runtime/policies/{conversation_id}` | `handlers_runtime::get_runtime_policy` / `upsert_runtime_policy` | `auto_mode`, `max_auto_turns`, `max_workflow_turns`, `require_human_after_n_turns`, `allow_agent_to_agent`, `allow_file_write`, `default_reviewer_agent_id`, `default_coordinator_agent_id`, `untagged_human_mode` |
| `GET /v1/conversations/{conversation_id}/runtime-status` | `handlers_runtime_status` | Per-agent `busy`/`queued`/`idle` |
| `POST /v1/companies/{company_id}/reset-sessions` | `handlers_runtime::reset_company_sessions` | Clears `external_session_id` on the company's bindings |
| `GET /v1/ws/terminals/{binding_id}?token=&cols=&rows=` | `handlers_terminals::websocket_terminal` | Bridges browser bytes to the PTY; `POST /v1/terminals/{binding_id}/ensure` and `POST .../input` reuse the same pool |
| `PUT`/`DELETE /v1/conversations/{conversation_id}/hide` | `handlers_conversations::hide_agent_session` / `restore_hidden_agent_session` | Per-user `conversation_hidden` row; agents are forbidden |
| `POST /v1/runtime-hosts/{host_id}/harness-accounts`, `POST .../harness-accounts/{account_id}/verify` | `handlers_runtime_hosts::register_harness_account`, `verify_harness_account` | Called by the runtime host |
| `POST /v1/companies/{company_id}/harness-accounts/{account_id}/logins`, `GET .../{login_id}`, `POST .../{login_id}/callback`, `POST .../{login_id}/cancel` | `handlers_harness_logins` | Browser side of a sign-in, on this device or a runtime host; cancel closes an open login so a new one can start before the 15-minute expiry |
| `POST /v1/runtime-hosts/{host_id}/harness-account-logins/claim`, `.../{login_id}/publish`, `.../{login_id}/callback/claim`, `.../{login_id}/complete`, `.../{login_id}/fail` | `handlers_harness_logins` | Connector side of a sign-in on a runtime host |
| `POST /v1/workspace-sessions/scan`, `POST /v1/workspace-sessions/import` | `handlers_workspace_sessions` | Scan native session stores, import selected sessions as terminal DMs |
| `GET /api/drivers/availability`, `GET /api/drivers/models`, `/api/harness-accounts[...]` | Next.js routes | Web-side driver and account surface |

Terminal path: `authorize_terminal_binding` requires a human caller, a terminal driver, a non-disabled binding, a direct conversation containing both caller and agent, an enabled agent and matching accessible workspaces; `ensure_pty_session` evicts dead children, reuses a live `PtySession`, or spawns the binary with `terminal_cli_args` (Codex adds `--config check_for_update_on_startup=false` and a managed `CODEX_HOME`), `TERM=xterm-256color`, `CHORUZ_SEND`, `DISABLE_AUTOUPDATER`, `PI_SKIP_VERSION_CHECK` and the harness account env. Codex sessions are captured by `begin_codex_terminal_capture` and anchored by `write_terminal_session_anchor` so a later `ensure_terminal` resumes the same native session.

Headless path: the pipeline's `spawn_headless_session` resolves the active binding, resumes `external_session_id` only when `session_provenance_matches(binding, "headless")` (Claude resumes on any non-empty id), and spawns `HeadlessDriver::args`. Web provisioning (`apps/web/lib/agents/agent-provisioning.ts`, `POST /api/agents/provision`) creates the principal, workspace, helper, instruction file and binding.

## Invariants

| Invariant | Pinned by |
|---|---|
| One non-disabled binding per agent (`agent_runtime_bindings_one_per_agent` partial unique index); a second create returns the existing one | `binding_defaults_and_uniqueness_are_enforced`, `binding_creation_waits_for_disable_and_rejects_the_disabled_agent` in [`crates/choruz-agent-runtime/tests/runtime_store.rs`](../../crates/choruz-agent-runtime/tests/runtime_store.rs) |
| Disabling an agent's bindings is atomic and idempotent | `disabling_agent_bindings_is_atomic_and_idempotent` |
| `workspace_path` is normalised and guarded; state transitions follow `can_transition_to` | `workspace_paths_are_normalized_and_guarded`, `state_transitions_are_guarded` |
| Binding state changes and rebinds write `audit_log` entries | `state_changes_and_rebind_write_audit_entries` |
| A terminal session anchor is accepted only for the binding generation that captured it and only for the same workspace | `terminal_session_anchor_preserves_unrelated_config_and_validates_binding`, `terminal_session_anchor_rejects_delayed_capture_after_reset_touch`, `codex_terminal_anchor_rejects_same_native_session_for_same_workspace_binding` |
| Terminal routes never write `conversation_events`; terminal bytes bypass the pipeline | `terminal_routes_do_not_write_conversation_events` in [`services/choruz-api-gateway/src/tests/runtime.rs`](../../services/choruz-api-gateway/src/tests/runtime.rs) |
| A binding may reference only an `active` harness account of the same company, driver and host, and only a model listed in `models_json`; the trigger stamps `harness_account_name` and `harness_account_profile_kind` | `validate_runtime_binding_harness_account` (V035); `harness_account_binding_trigger_rejects_unverified_models` |
| At most one open login per account; OAuth tokens never enter the database | `harness_account_login_open_account_idx`; table comments in V035 and V036 |
| Native import is idempotent per (`workspace_path`, `driver_type`, `native_session_id`) and serialised by an advisory lock key | `native_session_import_runs_end_to_end_and_is_idempotent`, `native_session_import_lock_key_executes_against_postgres` |
| Imported sessions are terminal bindings with an explicit `binary_path` | V032, V033 |
| Hiding a session is a per-user view preference that emits a `sync_change` and never stops the agent | `trg_conversation_hidden_sync_change` (V034); `api_restore_hidden_conversation` helper in `tests.rs` |
| PTY sessions persist until the child exits; there is no idle eviction | `evict_stale_pty_sessions` doc comment in [`state.rs`](../../services/choruz-api-gateway/src/state.rs) |
| A policy's `default_coordinator_agent_id` must be a valid principal | `policy_rejects_invalid_default_coordinator`, `policy_defaults_and_upsert_work` |

## Failure modes

| Failure | Behaviour | Signal |
|---|---|---|
| CLI binary not installed | terminal spawn fails; headless turn dead-letters with `driver_unavailable`; `GET /api/drivers/availability` reports `unavailable` with `setupHint` | availability endpoint, `executor_command_failed` |
| Gateway restart | PTY pool is lost; the next `ensure_terminal` respawns and reconciles Codex capture metadata | `PTY child process exited, recreating session` warn; `codex_terminal_open_reconciles_capture_metadata_after_gateway_restart_window` |
| Stale `external_session_id` | headless resume-failure phrases clear the id; `sync_session_id_from_disk` and `backfill_session_ids` repair from the native session store; `reset_company_sessions` clears a whole company | `not resuming CLI session` warn |
| Binding workspace missing on disk | headless turn fails without retry | `binding workspace_path does not exist on disk` |
| Harness authentication fails | `status` becomes `reauth_required` or `error`; a sign-in can end `failed`, `cancelled` or `expired` before a verified identity is stored | account list in the web UI |
| Model or quota refresh fails after authentication | the login remains `verified` and the account remains `active`; the last complete snapshot and `probed_at` are preserved | gateway or connector warning; account refresh action in the web UI |
| Terminal opened by a non-member, an agent, a group conversation or a disabled binding | 403 from `authorize_terminal_binding` | HTTP status |
| Lease lost during a headless turn | handled by the pipeline lease monitor (see [message-pipeline](message-pipeline.md)) | `lease expired` warn |

## Tests

- [`crates/choruz-agent-runtime/tests/runtime_store.rs`](../../crates/choruz-agent-runtime/tests/runtime_store.rs): PostgreSQL-backed `RuntimeStore` tests that apply `migrations/` themselves; unit tests in [`headless.rs`](../../crates/choruz-agent-runtime/src/headless.rs), [`session_catalog.rs`](../../crates/choruz-agent-runtime/src/session_catalog.rs), [`binding.rs`](../../crates/choruz-agent-runtime/src/binding.rs).
- [`crates/choruz-session/tests/integration.rs`](../../crates/choruz-session/tests/integration.rs) (`CHORUZ_TEST_DATABASE_URL`): sessions, leases, runtime status.
- [`crates/choruz-executor/src/wal.rs`](../../crates/choruz-executor/src/wal.rs), [`sandbox.rs`](../../crates/choruz-executor/src/sandbox.rs), [`adapter.rs`](../../crates/choruz-executor/src/adapter.rs), [`codex_adapter.rs`](../../crates/choruz-executor/src/codex_adapter.rs) test modules.
- [`services/choruz-api-gateway/src/tests/`](../../services/choruz-api-gateway/src/tests/): `terminal_routes_do_not_write_conversation_events`, `codex_terminal_open_reconciles_capture_metadata_after_gateway_restart_window`, `harness_account_binding_trigger_rejects_unverified_models`, `native_session_import_runs_end_to_end_and_is_idempotent`; unit tests in [`handlers_terminals.rs`](../../services/choruz-api-gateway/src/handlers_terminals.rs).
- Web: [`apps/web/lib/drivers/driver-availability.test.ts`](../../apps/web/lib/drivers/driver-availability.test.ts), [`apps/web/app/api/drivers/availability/route.test.ts`](../../apps/web/app/api/drivers/availability/route.test.ts), [`apps/web/app/api/harness-accounts/route.test.ts`](../../apps/web/app/api/harness-accounts/route.test.ts).
- End to end: [`apps/web/tests/e2e/terminal.spec.ts`](../../apps/web/tests/e2e/terminal.spec.ts), [`real-codex-driver.spec.ts`](../../apps/web/tests/e2e/real-codex-driver.spec.ts), [`workspace-session-import.spec.ts`](../../apps/web/tests/e2e/workspace-session-import.spec.ts), [`machines.spec.ts`](../../apps/web/tests/e2e/machines.spec.ts), [`agent.spec.ts`](../../apps/web/tests/e2e/agent.spec.ts).
- Manual smokes: [`docs/testing/real-driver-session-isolation-smoke.md`](../testing/real-driver-session-isolation-smoke.md), [`docs/testing/real-harness-platform-smoke.md`](../testing/real-harness-platform-smoke.md) with [`infra/host/smoke/real-harness-platform-smoke.ts`](../../infra/host/smoke/real-harness-platform-smoke.ts).

## Related

- [message-pipeline](message-pipeline.md): the dispatch and executor loops that consume bindings and leases.
- [agent-protocol](agent-protocol.md): the helper, outbox and instruction files installed into every workspace.
- [host-and-remote](host-and-remote.md): `runtime_host`, pairing and the connector that runs harness logins remotely.
- [choruz-api-gateway](api-gateway.md), [store](store.md), [web-client](web-client.md).
- Agent Notes: [Workspace-scoped isolation](../../.agents/notes/implemented/architecture/2026-08-18-workspace-scoped-isolation.md).
