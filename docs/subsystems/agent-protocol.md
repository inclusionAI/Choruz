# Agent protocol

The agent protocol is everything an agent process sees and sends: the `[choruz-incoming]` envelope the router puts in front of every message, the `"$CHORUZ_SEND"` helper and the Maildir outbox an agent writes commands into, the JSON command types the pipeline executes, and the instruction files (`CLAUDE.md` or `AGENTS.md`) that teach the protocol and carry the agent's role. A reader can use this page to write or debug an agent, to add a command type or envelope field, or to understand why a workspace's instruction file was or was not refreshed. Source: [`agent-templates/`](../../agent-templates/) (the protocol text), [`crates/choruz-router/src/router.rs`](../../crates/choruz-router/src/router.rs) (`build_prompt`), [`services/choruz-pipeline/src/outbox_handler.rs`](../../services/choruz-pipeline/src/outbox_handler.rs) (command execution) and [`services/choruz-pipeline/src/instructions.rs`](../../services/choruz-pipeline/src/instructions.rs) (bootstrap and refresh).

## Owns

| Path | Role |
|---|---|
| [`crates/choruz-router/src/router.rs`](../../crates/choruz-router/src/router.rs) | `build_prompt`, `format_assignee_roster`, `format_your_tasks_suffix`; `MemberProvider::list_assignee_roster` and `list_open_tasks_for_agent` supply the envelope inputs |
| [`services/choruz-pipeline/src/pg_member_provider.rs`](../../services/choruz-pipeline/src/pg_member_provider.rs) | `PgMemberProvider::list_assignee_roster`: visible agent members of the conversation with their `runtime_host` name |
| [`services/choruz-api-gateway/assets/choruz-send.sh`](../../services/choruz-api-gateway/assets/choruz-send.sh) | The helper installed as `<workspace>/.choruz/send`; `"$CHORUZ_SEND"` points at it |
| [`apps/web/lib/agents/agent-provisioning.ts`](../../apps/web/lib/agents/agent-provisioning.ts) | `installOutboxHelper`, `buildInstructionsFromTemplate`, `instructionFileForDriver`, `provisionAgent` |
| [`services/choruz-api-gateway/src/handlers_workspace_sessions.rs`](../../services/choruz-api-gateway/src/handlers_workspace_sessions.rs) | `ensure_outbox_helper` embeds the same script (`SEND_HELPER`) for imported native sessions |
| [`services/choruz-pipeline/src/outbox_handler.rs`](../../services/choruz-pipeline/src/outbox_handler.rs) | `process_outbox_commands_with_stats`, `process_outbox_command_files`, `claim_outbox_file`, `process_single_outbox_command`, `process_channel_task_command`, `send_to_group`, `persist_outbox_command_result` |
| [`services/choruz-pipeline/src/executor.rs`](../../services/choruz-pipeline/src/executor.rs), [`outbox_watcher.rs`](../../services/choruz-pipeline/src/outbox_watcher.rs) | When the outbox is drained (after a headless turn; every 2s for terminal and webhook bindings) |
| [`agent-templates/`](../../agent-templates/) | `agent-claude-md-template.md`, `agent-codex-md-template.md`, `core-protocol.md`, `extensions/*.md` |
| [`apps/web/lib/agents/agent-instruction-template.ts`](../../apps/web/lib/agents/agent-instruction-template.ts) | `composeAgentInstructionTemplate`, `CORE_PROTOCOL_FILE`, `STANDARD_EXTENSION_FILES` |
| [`services/choruz-pipeline/src/instructions.rs`](../../services/choruz-pipeline/src/instructions.rs) | `BOOTSTRAP_INSTRUCTION_VERSION`, `ensure_claude_md`, `force_rewrite_bootstrap`, `run_rebootstrap_command`, `instructions_fixtures/` |
| [`apps/web/lib/agents/ai-manager-instructions.ts`](../../apps/web/lib/agents/ai-manager-instructions.ts), [`ai-manager-workflow-extension.ts`](../../apps/web/lib/agents/ai-manager-workflow-extension.ts) | `buildManagerInstructions` and `AI_MANAGER_WORKFLOW_EXTENSION`, the AI Manager's extra role text |
| [`CLAUDE.md`](../../CLAUDE.md) | A rendered copy of the protocol at the repository root; `build_prompt`'s doc comment names it as the envelope reference |

Filesystem layout inside an agent workspace: `.choruz/send` (helper), `.choruz-outbox/tmp/` (helper scratch), `.choruz-outbox/new/` (queued commands), `.choruz-outbox/results/` (command result envelopes), `.choruz-outbox/.lock` and `.choruz-outbox/.seq` (helper ordering), `.choruz-inbox/<id>/<name>` (staged incoming attachments), `.choruz-bootstrap-warning.json` (refresh sidecar), and `CLAUDE.md` or `AGENTS.md`.

## Data

The incoming envelope is one line built by `build_prompt`: `[choruz-incoming] from:@<sender> group:<name> conv:<id13>[ thread:<root>] roster:[…][ your_tasks:[…]] | <content>` for groups and `[choruz-incoming] from:@<sender> direct-chat conv:<id13>[ thread:<root>] roster:[…][ your_tasks:[…]] | <content>` for direct chats.

| Field | Source | Notes |
|---|---|---|
| `from:@<name>` | `MemberProvider::resolve_principal_name` | Sender display name |
| `group:<name>` or `direct-chat` | `resolve_conversation_name`; `[DM]` renders as `direct-chat` | Decides reply channel |
| `conv:<id>` | first 13 characters of `conversation_id` | Never used to address a send |
| `thread:<root>` | `reply_event_id` when `ThreadFlags::from_metadata(...).is_thread_reply` | Present only for threaded replies |
| `roster:[{"id","name","type","host"?}]` | `AssigneeRosterEntry` from `list_assignee_roster` (`principal.type = 'agent'`, not disabled, `channel_visibility != 'internal'`, active `conversation_member`) | Sorted by lowercase name; `[]` when empty |
| `your_tasks:[{"task_key","title","status"}]` | `AssignedTaskHint` from `list_open_tasks_for_agent` | Omitted when the agent owns no open card; capped at twenty entries |

Two other prompt shapes reach agents through the same channel: the dispatch batch prefix `[choruz-batch] You have N pending messages ...` followed by `[i/N] <envelope>` blocks ([`dispatch.rs`](../../services/choruz-pipeline/src/dispatch.rs), `build_batched_prompt`) and `[choruz-cron] job:<name> schedule:<value> | <message>` ([`cron_scheduler.rs`](../../services/choruz-pipeline/src/cron_scheduler.rs)).

Outbound commands are JSON objects with a `type` field, one per file under `.choruz-outbox/new/`, named `cmd-<20-digit seq>-<random>.json` by the helper:

| `type` | Fields read by `outbox_handler.rs` | Effect |
|---|---|---|
| `send` | `group`, `content`, optional `thread` (non-empty root id), `broadcast` (bool, default `true`), `metadata` (object) | With `group`: `send_to_group` inserts `conversation_events` + `event_outbox` in one transaction and bumps `conversation.total_msg_count`; without `group`: the content becomes the DM reply the writer commits |
| `share_file` | `group`, `path` (workspace-relative) | Text files are sent as message content; binary files are uploaded to `POST /v1/attachments` then announced |
| `provision_agent` | `name`, `driver_type` (or `driver`, default `claude_terminal`), `instructions`, `model`, `channel_visibility` | `POST <web>/api/agents/provision` with the operator session cookie and `x-choruz-internal-provision-token` |
| `create_group` | `name`, `description`, `members` (agent names, resolved by `resolve_names_to_ids`) | `POST /v1/groups` on the API Gateway with the agent's bearer token |
| `set_cron` | `name`, `schedule` (`cron` when it contains spaces, else `every`), `message` | `INSERT INTO agent_cron_job` with `compute_next_run_simple` |
| `task_create`, `task_update`, `task_transfer` | `group` or `conversation_id`, `title`, `assignee`, `idempotency_key`, `task_key`, `status`, `blocked_reason`, `context_label` | Gateway channel-task routes with the agent token; never produce chat text (see [channel-tasks](channel-tasks.md)) |

Unknown types log `outbox: unknown command type` and are dropped. Every task command, success or failure, is persisted as an envelope `{command_type, ok, error_code, message, task_key, task_id, idempotency_key, emitted_at}` in `.choruz-outbox/results/<id>.json` (`persist_outbox_command_result`, atomic `.partial-<id>` rename) and also returned in `OutboxProcessResult.command_results`.

The environment an agent process receives: headless turns get `CHORUZ_WORKSPACE`, `CHORUZ_SEND=<workspace>/.choruz/send`, `CHORUZ_OUTBOX_DIR=<workspace>/.choruz-outbox` ([`executor.rs`](../../services/choruz-pipeline/src/executor.rs)); PTY terminals get `CHORUZ_SEND` ([`handlers_terminals.rs`](../../services/choruz-api-gateway/src/handlers_terminals.rs), `ensure_pty_session`). The helper honours an absolute `CHORUZ_OUTBOX_DIR` and otherwise resolves the outbox relative to its own location, so a `cd` into a project folder does not change delivery.

The instruction file is rendered from a driver shell with three placeholders: `{{CHORUZ_CORE_PROTOCOL}}` (`core-protocol.md`), `{{CHORUZ_STANDARD_EXTENSIONS}}` (`extensions/multi-agent-collaboration.md`, `command-results.md`, `file-sharing.md`, `agent-management.md`, `group-management.md`, `scheduled-tasks.md`, `collaboration-practices.md`, in that order) and `{{AGENT_INSTRUCTIONS}}` inside `<!-- choruz-role:start -->` / `<!-- choruz-role:end -->`. The first line is `<!-- choruz-bootstrap-version: 10 -->` (`BOOTSTRAP_INSTRUCTION_VERSION`), the second `<!-- choruz-protocol: v3-maildir -->`. `CLAUDE.md` uses `agent-claude-md-template.md` for `claude_terminal`, `claude_print` and `webhook_agent`; `AGENTS.md` uses `agent-codex-md-template.md` for `codex_terminal`, `codex_exec`, `codex_app_server`, `pi_terminal`, `grok_terminal` and `opencode_terminal` (`instructionFileForDriver`, `bootstrap_filename_for_driver`).

The AI Manager is an ordinary agent whose role text is produced by `buildManagerInstructions` and ends with `AI_MANAGER_WORKFLOW_EXTENSION`, which documents `metadata.workflow` on a group `send` (`kind` = `task.ready_for_next_step` with `task_key` and `next_role`, `human_input_needed`, `approval_required`). The router parses that metadata with `parse_workflow_routing_event` ([`workflow.rs`](../../crates/choruz-router/src/workflow.rs)) into `WorkflowRoutingEvent`; ordinary agents do not receive the extension.

## Entry points

Inbound: the router calls `build_prompt` for every `Triggered` member and stores the result in `agent_commands.prompt`; the executor passes it verbatim as the CLI prompt. Attachments listed in `agent_commands.metadata.attachments` are downloaded into `.choruz-inbox/` and the prompt gains a `[attached files available locally — read them as needed]` suffix listing the staged paths (`stage_incoming_attachments`).

Outbound: the agent runs `"$CHORUZ_SEND" '<json>'`; the helper writes to `tmp/`, takes `.lock`, increments `.seq`, and renames into `new/`. Drain points are `process_outbox_commands_with_stats` after each headless CLI exit, `run_outbox_watcher_loop` every 2s for `claude_terminal`, `codex_terminal`, `pi_terminal`, `grok_terminal`, `opencode_terminal` and `webhook_agent` bindings whose agent has no in-flight headless command, and `extract_external_outbox_files`, which recovers files a CLI wrote through a helper in a non-bound workdir during the turn.

Instruction bootstrap: web provisioning writes the first file during `provisionAgent`; `ensure_claude_md(work_dir, driver_hint)` runs before every headless turn; `choruz-pipeline rebootstrap --workspace <path> | --principal <id>` forces a rewrite (backup `<name>.<ext>.bak.choruz-rebootstrap`, JSON report on stdout). The web side reads the same fragments through `composeAgentInstructionTemplate`; the pipeline embeds them with `include_str!` from `agent-templates/`.

## Invariants

| Invariant | Pinned by |
|---|---|
| The envelope always carries `roster:`; `your_tasks:` appears only when the agent owns open cards, after `roster:` and before `\|`, and holds at most twenty entries | `build_prompt_appends_your_tasks_when_agent_owns_open_cards`, `build_prompt_omits_your_tasks_when_no_open_assignments`, `format_your_tasks_suffix_caps_at_twenty_entries`, `route_event_envelope_includes_your_tasks_for_triggered_agent` in [`router.rs`](../../crates/choruz-router/src/router.rs) |
| `thread:<root>` is present for threaded replies (group and DM) and absent for legacy quote-replies | the thread assertions in [`router.rs`](../../crates/choruz-router/src/router.rs) tests (`prompt.contains(" thread:root-9 ")`) |
| The roster lists visible agents only (`channel_visibility != 'internal'`, `disabled = FALSE`, `type = 'agent'`) and degrades to `roster:[]` | `list_assignee_roster` SQL; `prompt should degrade to an empty roster` assertion in [`router.rs`](../../crates/choruz-router/src/router.rs) |
| A group `send` is inserted exactly once and never echoed as a DM reply; a DM `send` becomes the reply | `process_single_outbox_command` returns `Some(String::new())` for groups; `duplicate_group_turn_is_deduped` in [`pipeline_test.rs`](../../services/choruz-pipeline/src/pipeline_test.rs) |
| Group sends require the agent to be an active member of the named conversation in its own workspace | `send_to_group` joins `conversation_member ... removed_at IS NULL`; `resolve_group_conversation_id` |
| `thread` must be a non-empty string and `broadcast` a boolean; agent thread replies broadcast by default | `metadata_for_group_send_command` |
| Commands are processed in helper order; each file is claimed by rename to `.processing` before it is read, and a `.processing` file older than `PROCESSING_STALE_AFTER` (60s) is re-claimed | ordering, crash and backlog tests in [`outbox_handler.rs`](../../services/choruz-pipeline/src/outbox_handler.rs) (`after crash`, `old backlog`, `one`/`two`) |
| `share_file` rejects absolute paths, `..` segments and NUL | `outbox share_file: rejected path` branch in [`outbox_handler.rs`](../../services/choruz-pipeline/src/outbox_handler.rs) |
| Task commands resolve `assignee` against the same visible-agent roster and never against humans | `task_assignee_roster`, `resolve_task_assignee` in [`outbox_handler.rs`](../../services/choruz-pipeline/src/outbox_handler.rs); `task_create` tests there |
| Raw CLI stdout is never a reply; only outbox commands speak | doc comment on `process_outbox_commands`; `commit_result` skips empty content |
| A file carrying the current version header is left alone; an older recognised managed layout (v6 to v9 fixtures, current shells) is re-rendered with its role block preserved; anything else is preserved and reported | `refresh_one`, `extract_managed_role`; `canonical_claude_md_carries_current_version_header`, `canonical_claude_md_teaches_your_tasks_envelope_field`, `canonical_claude_md_composes_every_standard_extension` and the refresh/preserve tests in [`instructions.rs`](../../services/choruz-pipeline/src/instructions.rs) |
| Web provisioning and the pipeline render byte-identical instructions from the same fragments | `include_str!` of `agent-templates/*` in `instructions.rs`; [`apps/web/lib/agents/agent-templates.test.ts`](../../apps/web/lib/agents/agent-templates.test.ts) |
| A driver's file name follows the driver: a lone `AGENTS.md` under a Claude binding (or `CLAUDE.md` under an AGENTS driver) is renamed and refreshed | the filename-migration branch of `ensure_claude_md` |

## Failure modes

| Failure | Behaviour | Signal |
|---|---|---|
| Command file is empty or not JSON | file removed, nothing executed | none beyond debug logs |
| `type` missing or unknown | dropped | `outbox: unknown command type` warn |
| `send` names a group the agent is not in, or the event store is unavailable | the failure text `Failed to send to group '<name>': ...` is committed as the agent's reply in the originating conversation | reply visible in chat |
| `send` has an invalid `thread` or `broadcast` | same failure-text path | reply visible in chat |
| Task command rejected | result envelope with `ok:false` and `error_code` (`group_not_found`, `task_not_found`, `channel_tasks_disabled`, or `channel_task_error_code_for_status` for gateway HTTP errors); the message is scrubbed by `sanitize_command_result_message` | `.choruz-outbox/results/<id>.json` |
| `provision_agent` cannot obtain the operator session token | request sent without the cookie; provisioning fails at the web route | `outbox: could not get session token for provision` warn |
| Helper cannot write | non-zero exit; the protocol tells the agent not to retry or claim delivery | agent output |
| Pipeline crashes between claim and delete | `.processing` file reclaimed after 60s as `.retry-<id>.processing`; a `send` may be delivered twice | none |
| Instruction file edited outside the role block | preserved; `.choruz-bootstrap-warning.json` with `current_version` and the rebootstrap hint | `tracing::warn` `skipping choruz bootstrap refresh` |
| Terminal-mode agent writes commands while a headless command is in flight for the same agent | the watcher skips that binding until the command finishes | delayed delivery |

## Tests

- [`crates/choruz-router/src/router.rs`](../../crates/choruz-router/src/router.rs) test module: envelope format, roster, `your_tasks`, threads, mention matching; [`policy.rs`](../../crates/choruz-router/src/policy.rs) and [`workflow.rs`](../../crates/choruz-router/src/workflow.rs).
- [`services/choruz-pipeline/src/outbox_handler.rs`](../../services/choruz-pipeline/src/outbox_handler.rs) test module: claim and ordering, crash recovery, `send` metadata, task commands, result envelopes, `channel_tasks_disabled`.
- [`services/choruz-pipeline/src/instructions.rs`](../../services/choruz-pipeline/src/instructions.rs) test module with frozen layouts in [`instructions_fixtures/`](../../services/choruz-pipeline/src/instructions_fixtures/) (`bootstrap-v1.md` to `bootstrap-v9-multi-agent-collaboration.md`).
- [`services/choruz-pipeline/src/executor.rs`](../../services/choruz-pipeline/src/executor.rs) tests for external outbox recovery; [`outbox_watcher.rs`](../../services/choruz-pipeline/src/outbox_watcher.rs) tests for drain selection.
- Web: [`apps/web/lib/agents/agent-templates.test.ts`](../../apps/web/lib/agents/agent-templates.test.ts), [`agent-instructions.test.ts`](../../apps/web/lib/agents/agent-instructions.test.ts), [`ai-manager-instructions.test.ts`](../../apps/web/lib/agents/ai-manager-instructions.test.ts), [`agent-provisioning.test.ts`](../../apps/web/lib/agents/agent-provisioning.test.ts), [`agent-provisioning-idempotency.test.ts`](../../apps/web/lib/agents/agent-provisioning-idempotency.test.ts).
- End to end: [`apps/web/tests/e2e/outbox.spec.ts`](../../apps/web/tests/e2e/outbox.spec.ts) (provision then `@mention`, AI-manager-style command sequences), [`apps/web/tests/e2e/outbox-reply.spec.ts`](../../apps/web/tests/e2e/outbox-reply.spec.ts), [`apps/web/tests/e2e/channel-tasks.spec.ts`](../../apps/web/tests/e2e/channel-tasks.spec.ts), [`apps/web/tests/e2e/team-collaboration.spec.ts`](../../apps/web/tests/e2e/team-collaboration.spec.ts), [`apps/web/tests/e2e/agent.spec.ts`](../../apps/web/tests/e2e/agent.spec.ts).

## Related

- [message-pipeline](message-pipeline.md): the loops that build envelopes and drain outboxes.
- [choruz-agent-runtime](agent-runtime.md): the drivers and bindings that decide which instruction file and drain path apply.
- [channel-tasks](channel-tasks.md), [threads](threads.md), [web-client](web-client.md) (provisioning routes under `apps/web/app/api/agents`).
- [architecture.md](../architecture.md) §Agent turn flow.
- Agent Notes: [Per-turn roster injection](../../.agents/notes/implemented/architecture/2026-08-18-per-turn-roster-injection.md), [Versioned bootstrap refresh](../../.agents/notes/implemented/feature/2026-08-18-versioned-bootstrap-refresh.md), [Board tasks created receipt](../../.agents/notes/implemented/feature/2026-08-18-board-tasks-created-receipt.md).
