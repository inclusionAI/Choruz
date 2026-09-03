# eChat Test Scenario Catalog

This catalog is the bridge between product requirements, automated tests, and
`BUGS.md`. It turns product behavior into scenario IDs that can be referenced by
Playwright specs, integration tests, manual probes, and bug rows.

## How to Use This Catalog

1. Pick a product area and scenario ID before writing a test.
2. Convert the scenario into the smallest durable repro, usually a focused
   Playwright spec under `apps/web/tests/e2e/` or a Rust integration test for
   runtime-only behavior.
3. If the scenario fails for product reasons, add or update a `BUGS.md` row and
   include the scenario ID in the `Repro Artifact` or `Notes` column.
4. When fixed, keep the test as regression coverage and update the scenario's
   automation status in this file.

## Scenario ID Convention

| Prefix | Product Area |
| --- | --- |
| `AUTH` | Login, sessions, authorization, logout |
| `COMP` | Companies, workspaces, switching, isolation |
| `AGENT` | Agent provisioning, runtime binding, direct agent chats |
| `TEAM` | Multi-agent group tasks, handoffs, artifacts, and reviews |
| `CHAT` | Group and direct messaging, message rendering, replies |
| `MENTION` | Agent mention parsing, routing, and handoff chains |
| `OUTBOX` | `.choruz/send`, agent commands, provisioning from agents |
| `RT` | WebSocket, polling fallback, unread counts, persistence |
| `FILE` | File explorer, editor tabs, save/share/download behavior |
| `ATTACH` | Attachment upload, download, inline rendering |
| `SEARCH` | Sidebar and detail-panel search |
| `CRON` | Scheduled agent tasks |
| `VOICE` | Push-to-talk voice input and transcription UX |
| `SERVER` | Remote server manager and deployment controls |
| `VIZ` | Pixel World and visual agent status surfaces |
| `GIT` | Git graph and repository inspection surfaces |
| `TEMPLATE` | Curated agent and team templates |
| `DOCS` | In-app documentation pages |
| `TELEM` | Analytics and tracing |
| `UX` | Layout, keyboard, accessibility, responsive behavior |
| `OBS` | Health, metrics, logs, diagnostics |

Priorities:

- `P0`: core collaboration path or data isolation risk.
- `P1`: important feature behavior with user-visible failure.
- `P2`: polish, discoverability, or lower-risk workflow.

Automation status:

- `Covered`: a durable test already exists.
- `Partial`: nearby coverage exists, but an important assertion is missing.
- `Gap`: no durable automated coverage yet.
- `Manual`: keep as manual until the acceptance criterion is clearer.

## Source Documents

Primary sources for this first catalog pass:

- `docs/architecture.md`
- [`docs/data-model.md`](../data-model.md)
- `docs/bug-fixing-operating-model.md`
- the current Playwright workflow and browser tests under `apps/web/tests/e2e/**`
- `docs/testing/test-matrix.md`
- `docs/testing/golden-dataset.md`
- `apps/web/app/docs/**`
- `apps/web/tests/e2e/**`
- `BUGS.md`

The manually referenced file `Manual Testing of eChat.md` is not tracked in
this repository, so its cases should be folded in later if it becomes
available.

## P0 Smoke Set

Run this set before claiming the main product path is healthy.

| ID | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- |
| `AUTH-001` | Operator signs in with valid credentials. | Login succeeds, session cookie/token is stored, dashboard loads, and the sidebar shows the principal. | Covered: `apps/web/tests/e2e/auth.spec.ts` |
| `COMP-001` | User creates a company. | Company appears in the selector, has a unique workspace, and the creator is owner. | Covered: `apps/web/tests/e2e/company.spec.ts`, `apps/web/tests/e2e/user-journeys.spec.ts` |
| `COMP-003` | User creates a company with AI Manager enabled. | Company, `ai-manager` agent, PTY-backed AI Manager direct conversation, and workspace are created together. | Partial: `apps/web/tests/e2e/user-journeys.spec.ts`; strengthen with AI Manager terminal assertion |
| `AGENT-001` | User provisions an agent. | Agent principal, secret, direct conversation, runtime binding, and workspace are created. | Covered: `apps/web/tests/e2e/agent.spec.ts`, `apps/web/tests/e2e/api-routes.spec.ts` |
| `AGENT-010` | Supported agent drivers can be added to the platform. | Codex terminal/headless, Claude, and webhook agents can be provisioned with the correct binding, mode, workspace, direct chat surface, and visible error state. | Partial: executor driver matrix is covered by `services/choruz-pipeline/src/executor.rs`; add API/UI provisioning inspection and environment-gated real-driver smoke later |
| `TEAM-001` | User assigns a complex task to a mixed-driver multi-agent group. | Each eligible agent receives the intended task context, at least two driver families contribute distinct results, and the transcript/artifacts can be inspected afterward. | Partial: `apps/web/tests/e2e/team-collaboration.spec.ts` covers deterministic multi-agent delivery, distinct results, and transcript inspection; mixed Claude/Codex smoke remains environment-gated |
| `CHAT-001` | User sends a group message. | Message is persisted once, visible in UI/API, and ordered by sequence. | Covered: `apps/web/tests/e2e/messaging.spec.ts`, `apps/web/tests/e2e/outbox.spec.ts` |
| `MENTION-001` | User mentions one group agent. | Only the exact active member agent is routed and triggered. | Covered: `services/choruz-pipeline/src/pipeline_test.rs` for B-001; partial web coverage in `outbox.spec.ts` |
| `OUTBOX-001` | Agent sends text to a group through `.choruz/send`. | Message arrives in the named group and does not require direct outbox file edits. | Covered: `services/choruz-pipeline/src/outbox_handler.rs::process_outbox_commands_delivers_group_send_to_named_group`; broader UI outbox coverage remains in `apps/web/tests/e2e/outbox.spec.ts` |
| `OUTBOX-002` | AI Manager creates agents and then creates their group. | Agent names resolve to IDs, commands execute in order, and the generated group appears in the company sidebar. | Covered: `apps/web/tests/e2e/outbox.spec.ts` for B-002 |
| `RT-001` | Message arrives while another conversation is open. | Unread count increments, current conversation does not lose history, and opening the target clears unread. | Covered/Partial: `apps/web/tests/e2e/websocket.spec.ts`, `apps/web/tests/e2e/conversation.spec.ts` |
| `AGENT-004` | Operator can converse with AI Manager. | AI Manager DM opens the expected PTY terminal surface, accepts input, and can produce useful responses. | Partial: `apps/web/tests/e2e/user-journeys.spec.ts`; strengthen with terminal input assertion |
| `ATTACH-002` | User uploads a screenshot from chat input. | Group composers expose an upload affordance and send an attachment-backed message. | Covered: `apps/web/tests/e2e/messaging.spec.ts`; `I-001` is closed |
| `SEARCH-004` | Search isolation holds across conversations. | Users only see search results from conversations they can access. | Covered: `services/choruz-api-gateway/src/tests/::search_messages_isolation` |
| `COMP-002` | Workspace isolation holds across companies. | Cross-user companies, conversations, messages, and search results remain scoped. | Partial: focused company metadata, explicit workspace group creation, company-workspace send, personal conversation, read, send, and search isolation are covered by `apps/web/tests/e2e/workspace-isolation.spec.ts`; see fixed `B-013`, `B-014`, and `B-015`; add files/attachments/active-company UI coverage |

## Authentication and Sessions

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `AUTH-001` | P0 | Valid login. | API returns a session token and the web app opens `/dashboard`. | Covered: `auth.spec.ts` |
| `AUTH-002` | P0 | Invalid login. | Bad credentials are rejected without creating a session. | Covered: `auth.spec.ts`, `api-routes.spec.ts` |
| `AUTH-003` | P1 | Session reload. | Reload keeps the user authenticated and restores dashboard state. | Covered: `auth.spec.ts` |
| `AUTH-004` | P1 | Unauthenticated access. | Direct navigation to protected pages redirects to login or returns `401` for API calls. | Covered: `auth.spec.ts` |
| `AUTH-005` | P1 | Logout. | Session is cleared and protected resources are inaccessible afterward. | Covered: `auth.spec.ts` |
| `AUTH-006` | P0 | Token leakage guard. | Analytics, logs, and visible UI never expose the session token. | Partial: `telemetry.spec.ts`; add log/screenshot checks if a leak is suspected |
| `AUTH-007` | P1 | Expired or malformed token. | API returns `401`, the web app clears the invalid cookie, recovers to login, and no stale principal data remains visible. | Covered: `apps/web/tests/e2e/auth.spec.ts`, `apps/web/app/auth/session-invalid/route.test.ts` |
| `AUTH-008` | P0 | Non-operator company creation. | A normal authenticated human can create a company when product policy allows it. | Manual/currently historical: `B-012`; add durable API regression |

## Companies and Workspaces

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `COMP-001` | P0 | Create company. | New company appears in selector and `/v1/companies`; owner can switch into it. | Covered: `company.spec.ts`, `api-routes.spec.ts` |
| `COMP-002` | P0 | Workspace isolation. | Conversations, agents, files, and search results from Company A are invisible in Company B unless explicitly allowed. | Partial: `workspace-isolation.spec.ts` now covers fixed `B-013`, `B-014`, and `B-015` plus personal conversation/read/send/search isolation; add files/attachments/active-company UI coverage |
| `COMP-003` | P0 | Create company with AI Manager. | AI Manager agent and PTY-backed DM are auto-created under the new workspace. | Covered: `company.spec.ts` verifies UI creation, workspace-scoped agent, direct conversation, runtime binding, and PTY surface |
| `COMP-004` | P1 | Company switching. | Sidebar, active conversation, file explorer, and detail panel all refresh to the selected company. | Covered/Partial: `company.spec.ts` verifies workspace-only sidebar content and clears a stale active terminal; add detail-panel state coverage |
| `COMP-005` | P1 | Folder path selection. | Folder picker writes the chosen absolute path to the company configuration. | Manual/currently historical: `B-003` |
| `COMP-006` | P1 | Batch pause/resume agents. | Company-level agent activity toggle prevents and restores execution without losing conversations. | Gap |
| `COMP-007` | P1 | Rename/archive/delete company. | Company lifecycle actions update the selector, active company, and inaccessible archived/deleted resources. | Partial: `company.spec.ts`; add API state assertions |
| `COMP-008` | P0 | Owner exception boundary. | The company owner can access owned companies but cannot use that exception to leak unrelated workspaces. | Covered: `services/choruz-api-gateway/src/tests/::company_workspace_authorization_guards_hold` |

## Agents and Direct Conversations

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `AGENT-001` | P0 | Provision agent. | Principal, secret, workspace, direct conversation, and binding exist. | Covered: `agent.spec.ts`, `api-routes.spec.ts` |
| `AGENT-002` | P0 | Open agent DM. | Terminal-mode agents, including AI Manager, show a PTY surface that connects, focuses, and handles input. | Partial: `terminal.spec.ts`, `user-journeys.spec.ts`; strengthen with a real input assertion |
| `AGENT-003` | P1 | Agent instructions persist. | Instructions are saved, visible in settings/detail view, and included in runtime context. | Gap |
| `AGENT-004` | P0 | AI Manager DM conversation. | Operator can use the AI Manager through direct PTY terminal mode; no normal chat composer is required. | Partial: `user-journeys.spec.ts`; add positive PTY input/response regression |
| `AGENT-005` | P1 | Disabled agent filtering. | Disabled agents do not appear as active conversation targets and are not routed. | Covered/Partial: `agent.spec.ts`; add routing assertion |
| `AGENT-006` | P1 | Resume all agents. | Resume action restarts recoverable runtime bindings and reports failures visibly. | Partial: `agent.spec.ts`; add runtime verification |
| `AGENT-007` | P0 | Agent session isolation. | An agent never receives another agent's direct chat history or workspace context. | Covered/Partial: router direct-prompt history scoping, fake Codex foreign-session rejection, terminal resume wrong-binding rejection, Codex disk non-guessing, and Claude workspace-scoped local session-store lookup are covered; live Codex smoke passed with `CHORUZ_REAL_DRIVER_SMOKE=1 infra/host/smoke/agent-session-isolation-real-driver.sh` on 2026-05-17. Residual: live smoke covers direct Codex CLI session persistence/cwd isolation, not Choruz API provisioning or PTY WebSocket plumbing; Claude live smoke was not completed locally. |
| `AGENT-008` | P1 | Driver failure visibility. | Missing CLI, failed start, or bad runtime binding surfaces an actionable error in UI/API and does not retry forever. | Partial: `services/choruz-pipeline/src/executor.rs::claude_and_codex_failures_are_classified_for_bounded_recovery` covers missing binaries and authentication failures as non-retriable plus crashes and hangs as bounded retries; add UI error-surface coverage |
| `AGENT-009` | P1 | Agent secret rotation. | Rotating an agent secret invalidates the old secret and keeps the active binding recoverable. | Covered: `agent.spec.ts` verifies the old secret is rejected and the rotated secret can still publish through the existing agent binding. |
| `AGENT-010` | P0 | Driver compatibility matrix. | Codex terminal, Codex headless/pipeline, Claude, and webhook agents can each be provisioned, listed, opened, and inspected with the correct runtime binding and workspace. | Partial: `services/choruz-pipeline/src/executor.rs::supported_cli_driver_bindings_execute_with_fake_binaries` covers deterministic CLI execution paths, and `claude_and_codex_execute_concurrently_without_crossing_results` covers concurrent CLI isolation; add API/UI provisioning inspection and real-driver smoke only where local CLIs/webhook endpoints are configured |
| `AGENT-011` | P0 | Webhook agent execution. | A webhook-backed agent receives routed task payloads, returns an agent message, records provenance, and handles timeout/non-2xx responses visibly. | Partial: `services/choruz-pipeline/src/executor.rs::webhook_agent_binding_skips_cli_and_succeeds_empty` covers webhook no-CLI execution contract and `B-019`; add full delivery, payload, timeout, and non-2xx integration coverage |
| `AGENT-012` | P1 | Driver mode parity. | Each driver clearly supports or rejects terminal direct chat, headless group execution, mention routing, file sharing, and schedule execution according to its capabilities. | Gap: matrix assertion across driver capabilities |
| `AGENT-013` | P1 | Driver unavailable recovery. | Missing Claude/Codex CLIs, bad webhook URLs, and stopped bindings report actionable status without corrupting the agent principal or retrying forever. | Partial: deterministic Claude/Codex missing-binary, authentication, crash, and timeout behavior is covered by `services/choruz-pipeline/src/executor.rs::claude_and_codex_failures_are_classified_for_bounded_recovery`; add webhook and stopped-binding cases |
| `AGENT-014` | P0 | Multiple Harness accounts on one device. | A verified Claude Code or Codex identity activates its account independently of the model and quota refresh; a failed refresh preserves the last complete snapshot and cannot turn successful OAuth into a failed login. A complete refresh exposes exact identity, named quota windows, reset times, and account-specific models; provisioning binds the selected account and verified model without falling back to another login. Local and remote Codex accounts use browser OAuth, credentials stay in the selected device-local profile, and direct/group UI identifies the account that ran the Agent. Removing an account stops bindings that depend on it. | Covered: `apps/web/lib/agents/harness-accounts.test.ts`, `apps/web/lib/agents/agent-provisioning.test.ts`, `apps/web/app/api/harness-accounts/[id]/route.test.ts`, `services/choruz-api-gateway/src/tests/::harness_account_binding_trigger_rejects_unverified_models`, `::harness_logins::*` (Claude's complete `code#state` callback, current `value` model field, identity-first completion, catalog failure isolation, Codex browser completion, isolated profiles, failure state), `crates/choruz-harness-login` (current and legacy Claude model fields, remote loopback callback validation, quota bucket labeling), and `apps/web/tests/e2e/modals.spec.ts` (verified account UI without a catalog, local and remote browser handoff, and real dependent-binding removal); real Claude Code and Codex account provisioning, exact quota discovery, model selection, isolated login, and PTY replies verified locally on 2026-09-02. Add an opt-in repeatable live account smoke before making this a release gate. |

Driver compatibility automation should separate deterministic platform contracts
from real model behavior. PR-blocking tests should use fake or stubbed driver
bindings so they can assert provisioning, routing, payload shape, status, and
message persistence without depending on network, model output, or a developer's
local CLI installation. Real Codex, Claude, and webhook smoke tests are
still valuable, but should run as opt-in, nightly, or environment-gated checks.

## Team Tasks and Multi-Agent Collaboration

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `TEAM-001` | P0 | Complex mixed-driver task happy path. | A human asks a group containing at least two supported driver families to complete a multi-step task; each eligible agent receives the intended context, replies once or according to policy, and the final transcript is inspectable. | Partial: `team-collaboration.spec.ts` covers deterministic multi-agent fanout, distinct results, and transcript inspection with webhook fake agents; mixed Claude/Codex real-driver smoke remains environment-gated |
| `TEAM-002` | P0 | Multi-agent routing and fanout. | Mentions, group membership, disabled state, and workspace scope determine exactly which agents receive the task; no duplicate or ineligible route decisions are produced. | Covered: `team-collaboration.spec.ts` verifies exact eligible/disabled multi-mention delivery, while pipeline router/provider tests cover group and workspace exclusion |
| `TEAM-003` | P1 | Agent handoff and review loop. | One agent can mention another eligible group agent for follow-up review, and the handoff is auditable without leaking hidden context. | Covered: `team-collaboration.spec.ts` verifies persisted source and handoff messages plus an inspectable review response |
| `TEAM-004` | P1 | Inspect each agent's result. | The user can inspect each agent response, generated file, shared artifact, status, and error independently from the group transcript. | Partial: `team-collaboration.spec.ts` covers independent responses in the group transcript; file/artifact and per-leg error inspection remain gaps |
| `TEAM-005` | P1 | Partial failure handling. | If one agent is offline, times out, or returns an invalid command, other agents still complete and the failed leg is visible to the user. | Partial: `team-collaboration.spec.ts` proves a healthy webhook leg completes once while a non-2xx leg remains retriable; user-visible failed-leg status remains a gap |
| `TEAM-006` | P1 | Long multi-turn persistence regression guard. | A multi-turn task remains coherent after reload, WebSocket reconnect, polling fallback, sidebar navigation, and reopening the group, guarding against transcript/state regressions. | Partial: `team-collaboration.spec.ts` covers multiple agent results and hard reload; explicit reconnect, polling fallback, and sidebar navigation remain gaps |
| `TEAM-007` | P0 | Multi-agent privacy boundary. | Agents only receive messages, files, workspace paths, and prior context they are authorized to see, even during handoffs or artifact review. | Partial: `services/choruz-api-gateway/src/tests/::agent_privacy_surfaces_are_scoped_to_authorized_workspace_context` covers API surfaces and `team-collaboration.spec.ts` first confirms private source persistence, then verifies a handoff webhook excludes it; pipeline prior-context and attachment handoff coverage remain gaps |
| `TEAM-008` | P2 | Real model variability boundary. | Non-deterministic model wording does not make core platform tests flaky; assertions focus on routing, persistence, status, and artifact availability. | Manual/process: document alongside the current Playwright workflow if real-driver smokes are added |

## Chat and Message Behavior

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `CHAT-001` | P0 | Group message send. | Sending from UI/API creates exactly one message in the selected group. | Covered: `messaging.spec.ts`, `outbox.spec.ts` |
| `CHAT-002` | P0 | Message ordering. | Messages render and API-sort by monotonic sequence, including rapid sends. | Covered: `outbox.spec.ts` |
| `CHAT-003` | P0 | Idempotent retry. | Reusing the same idempotency key does not create duplicate messages. | Covered: `messaging.spec.ts` retries the exact API payload, verifies the same message ID, and asserts exactly one persisted message. |
| `CHAT-004` | P1 | Optimistic send reconciliation. | Optimistic UI message is replaced by server confirmation without duplicates. | Covered: `websocket.spec.ts`, `messaging.spec.ts` |
| `CHAT-005` | P1 | Markdown rendering. | Markdown, links, code blocks, and lists render without leaking raw control tags. | Covered: `messaging.spec.ts`, `message-list.spec.ts` |
| `CHAT-006` | P1 | Reply/quote. | Reply preview is shown, sent with context, and links visually to the source message. | Partial: `messaging.spec.ts`; add persistence/API assertion |
| `CHAT-007` | P1 | Edit/delete messages. | Editable own messages update in real time; deleted messages disappear for all clients. | Gap |
| `CHAT-008` | P0 | Group vs direct behavior. | Group conversations show the chat composer; direct terminal-mode bindings show the PTY surface. | Partial: `terminal.spec.ts`; strengthened with a focused direct xterm vs group-with-agent transcript/composer assertion, pending host e2e verification |
| `CHAT-009` | P1 | Manage chats. | Manage mode enters, selects, cancels, and deletes without corrupting the sidebar. | Covered: `sidebar.spec.ts`; historical `B-004` |
| `CHAT-010` | P0 | Conversation membership guard. | Removed or non-member principals cannot read, send, search, or receive realtime updates for the conversation. | Covered/Partial: `services/choruz-api-gateway/src/tests/::removed_and_never_members_cannot_access_conversation_surfaces`, `crates/choruz-fanout/src/gateway.rs::fanout_stops_sending_to_removed_member`, and `replay_denies_removed_member_with_stale_cursor` cover API and fanout guards; add browser-level e2e only if UI regressions recur |
| `CHAT-011` | P1 | Empty and oversized messages. | Empty messages are blocked; oversized or multiline messages fail gracefully or persist exactly as specified. | Partial: `messaging.spec.ts`, `keyboard.spec.ts`; add API boundary tests |
| `CHAT-012` | P1 | Special content rendering. | ANSI escapes, raw legacy reply tags, HTML, and malformed markdown do not break layout or leak unsafe markup. | Covered: `outbox.spec.ts`, `message-list.spec.ts`, and `messaging.spec.ts`, including an inert-HTML/XSS assertion. |

## Mentions and Routing

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `MENTION-001` | P0 | Exact mention match. | `@dev` triggers `dev`, not `dev2`; case-insensitive exact display-name matching works. | Covered: pipeline tests for `B-001` |
| `MENTION-002` | P0 | Mention membership guard. | Mentioning an agent outside the group or workspace does not route a command. | Covered/Partial: `services/choruz-pipeline/src/pipeline_test.rs::mentioning_agent_outside_group_does_not_generate_command` and `pg_member_provider::tests::cross_workspace_agent_member_is_not_routed_even_when_mentioned`; add web/API smoke only if mention UI membership filtering changes |
| `MENTION-003` | P1 | Mention dropdown. | Typing `@` lists eligible agents and inserts the selected mention. | Covered: `messaging.spec.ts`, `keyboard.spec.ts` |
| `MENTION-004` | P1 | No accidental trigger. | Plain text references without `@` do not wake agents. | Gap: add router/integration assertion |
| `MENTION-005` | P1 | Multi-mention fanout. | Mentioning multiple eligible agents creates one route decision per agent and no duplicates. | Covered: `team-collaboration.spec.ts` |
| `MENTION-006` | P1 | Agent handoff chain. | An agent reply mentioning another agent can route the next agent when policy allows it. | Covered: `team-collaboration.spec.ts` |
| `MENTION-007` | P1 | Ambiguous or renamed display names. | Duplicate, renamed, spaced, and punctuation-heavy names resolve deterministically or return a visible error. | Gap |
| `MENTION-008` | P0 | Disabled/offline mention. | Disabled agents and paused company agents are skipped with an auditable route decision. | Partial: `team-collaboration.spec.ts` verifies disabled agents receive no app-mention delivery; paused-company behavior and route-decision audit remain gaps |

## Outbox and Agent Commands

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `OUTBOX-001` | P0 | `.choruz/send` group text. | Agent message is delivered to the named group and visible to members. | Covered: `outbox_handler.rs::process_outbox_commands_delivers_group_send_to_named_group` plus broader `outbox.spec.ts` workflow coverage |
| `OUTBOX-002` | P0 | Provision agents then group. | `provision_agent` commands complete before `create_group` resolves generated names. | Covered: `outbox.spec.ts` for `B-002` |
| `OUTBOX-003` | P1 | Share file. | `share_file` attaches or pins the file in the conversation using the correct path. | Gap |
| `OUTBOX-004` | P1 | Create group with name resolution. | Member names resolve to principals in the current workspace only. | Partial: `outbox.spec.ts`; add cross-workspace negative case |
| `OUTBOX-005` | P1 | Invalid command. | Missing `type`, bad group, or malformed JSON creates a visible error/dead-letter without crashing the pipeline. | Gap |
| `OUTBOX-006` | P1 | Command ordering. | Multiple `.choruz/send` calls are processed independently and in intended order. | Partial: `outbox.spec.ts`; strengthen with ordering audit |
| `OUTBOX-007` | P0 | Group name vs conversation ID routing. | Agents must address groups by group name in Choruz protocol commands; UUID misuse fails visibly. | Covered/Partial: outbox handler tests cover UUID-shaped group names, member-scoped group-name routing, in-transaction membership recheck, missing/ambiguous group visible errors, Maildir single-claim concurrency, claim mtime refresh, stale claim recovery, multi-reply preservation, and watcher `set_cron` conversation resolution; watcher tests cover PTY-only draining, active-member guard, and visible reply publishing |
| `OUTBOX-008` | P1 | Duplicate command idempotency. | Replayed outbox commands do not duplicate groups, agents, cron jobs, or messages beyond the protocol contract. | Gap |

## Realtime, Persistence, and Recovery

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `RT-001` | P0 | WebSocket delivery. | Connected clients receive new messages without refresh. | Covered: `websocket.spec.ts` |
| `RT-002` | P0 | Polling fallback. | If WebSocket is unavailable, polling still loads new messages. | Covered: `websocket.spec.ts` |
| `RT-003` | P1 | Snapshot refresh. | Refreshing console state does not wipe visible message history. | Covered: `websocket.spec.ts` |
| `RT-004` | P1 | IndexedDB cache. | Messages persist locally and recover after hard reload without stale duplicates. | Covered: `indexeddb.spec.ts` |
| `RT-005` | P1 | Read/unread counts. | Counts increment off-screen and clear only when the conversation is viewed. | Covered/Partial: `conversation.spec.ts`, `websocket.spec.ts` |
| `RT-006` | P0 | Pipeline backlog. | Event backlog drains under normal traffic and dead letters are diagnosable. | Covered/Partial: router backlog/dead-letter tests plus `crates/choruz-session/tests/integration.rs::expired_batch_members_share_one_epoch_fence_and_all_retry` and `stale_attempt_cannot_overwrite_reassigned_command_or_heartbeat` cover lease recovery and stale-owner fencing; full host smoke remains a residual gap |
| `RT-007` | P1 | Reconnect cursor replay. | After disconnect/reconnect, the client receives missed events once and resumes from the correct cursor. | Gap |
| `RT-008` | P1 | Multi-tab consistency. | Two browser tabs for the same user converge on the same active messages, unread state, and cache contents. | Partial: `websocket.spec.ts` verifies exact-once active-message convergence in two tabs; cross-tab unread and cache convergence remain gaps. |

## Files, Editor, and Workspace Tools

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `FILE-001` | P1 | File tree loads. | Workspace tree renders folders/files for a company or agent workspace. | Covered: `file-explorer.spec.ts` |
| `FILE-002` | P1 | Open file in editor. | Clicking a file opens one editor tab with content and syntax highlighting. | Covered: `file-editor.spec.ts`, `editor-tabs.spec.ts` |
| `FILE-003` | P1 | Edit/save file. | Dirty state appears after edit, save persists, and dirty state clears. | Covered: `file-editor.spec.ts`, `user-journeys.spec.ts` |
| `FILE-004` | P1 | File tab navigation. | Conversation and file tabs can coexist, switch, reload, and close without duplicate tabs. | Covered: `editor-tabs.spec.ts` |
| `FILE-005` | P1 | Share file to chat. | Shared file content appears in the selected conversation and respects access boundaries. | Gap |
| `FILE-006` | P0 | Workspace path guard. | File APIs cannot read/write outside allowed workspace roots. | Partial: `apps/web/app/api/filesystem/route.test.ts` and `apps/web/lib/workspace/workspace-path-guard.test.ts` cover required `workspace_id`, workspace-scoped file read/write rejection, and symlink escape rejection for `B-022`; folder-picker browsing without a selected workspace remains governed by backend browse-root policy |
| `FILE-007` | P1 | Binary and large files. | Binary files do not corrupt the editor; large files show a safe preview or refusal path. | Gap |
| `FILE-008` | P1 | Agent-created file refresh. | Files created by an agent become visible after refresh or realtime update without switching companies. | Gap |

## Attachments

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `ATTACH-001` | P1 | Upload through API. | `POST /v1/attachments` stores file metadata and bytes. | Covered: `attachment.spec.ts` |
| `ATTACH-002` | P1 | Upload from chat input. | Chat input exposes an upload affordance, hidden file input, and attachment message publish path. | Covered: `messaging.spec.ts`; historical `I-001` |
| `ATTACH-003` | P1 | Inline image rendering. | Markdown image pointing at an attachment renders inline and can be inspected. | Covered/Partial: `attachment.spec.ts` |
| `ATTACH-004` | P1 | Authenticated download. | Attachment download/proxy requires a valid session and preserves content type. | Gap |
| `ATTACH-005` | P1 | Drag-and-drop. | Dragging files over the chat area does not crash and can start upload when enabled. | Partial: `attachment.spec.ts` |
| `ATTACH-006` | P1 | Attachment access isolation. | Users cannot fetch attachments from conversations or workspaces they cannot access. | Gap |
| `ATTACH-007` | P1 | Unsupported or oversized upload. | Unsupported type, empty file, and oversized payload produce clear errors without broken message bubbles. | Gap |

## Search

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `SEARCH-001` | P1 | Sidebar filter. | Sidebar search filters conversations and can be cleared. | Covered: `search.spec.ts`, `conversation.spec.ts` |
| `SEARCH-002` | P1 | Message full-text search. | Query returns accessible conversation messages, sorted by relevance. | Covered/Partial: `search.spec.ts`; add API ranking assertions |
| `SEARCH-003` | P1 | Result navigation. | Clicking a result opens the conversation, scrolls to the message, and highlights it. | Covered/Partial: `search.spec.ts` verifies result rendering, highlighting, click-through, and transcript visibility; cross-conversation navigation remains a gap because detail search is conversation-scoped. |
| `SEARCH-004` | P0 | Search isolation. | Results never include inaccessible workspaces or conversations. | Covered: `services/choruz-api-gateway/src/tests/::search_messages_isolation` |
| `SEARCH-005` | P2 | Minimum query length. | Queries below the minimum show no misleading results and no API error. | Gap |
| `SEARCH-006` | P1 | Newly sent message indexing. | A just-sent message is searchable immediately through UI and API. | Covered: `search.spec.ts` verifies the newly sent message ID through the API and the same content in the detail-search UI without an indexing delay. |
| `SEARCH-007` | P1 | Deleted/edited message indexing. | Deleted messages disappear from results; edited messages update their searchable content. | Gap |

## Cron and Scheduled Work

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `CRON-001` | P1 | Schedule tab visibility. | Agent direct conversations expose a Schedule tab. | Covered: `cron.spec.ts`, `detail-panel.spec.ts` |
| `CRON-002` | P1 | Create schedule. | UI/API/outbox can create an enabled job with a valid interval or cron expression. | Partial: `cron.spec.ts` verifies the API write and enabled persisted state; outbox creation remains a gap. |
| `CRON-003` | P1 | Toggle/delete schedule. | Disabled jobs do not run; deleted jobs disappear and stop running. | Partial: `cron.spec.ts` verifies persisted disable and deletion through the API; scheduler non-execution remains a gap. |
| `CRON-004` | P0 | Due job execution. | A due job inserts a message and triggers the target agent through the normal pipeline. | Covered/Partial: `services/choruz-pipeline/src/cron_scheduler.rs::due_cron_job_inserts_message_and_agent_command`; full executor smoke remains a residual gap |
| `CRON-005` | P1 | Invalid schedule. | Bad schedule strings show validation errors and do not create jobs. | Gap |
| `CRON-006` | P1 | Duplicate due processing. | A due job is not executed twice across scheduler restarts or retries. | Gap |

## Voice Input

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `VOICE-001` | P1 | Mic affordance. | Chat input shows a push-to-talk control with accessible title/label. | Covered: `voice-input.spec.ts` |
| `VOICE-002` | P1 | Browser without SpeechRecognition. | Clicking the mic without browser speech support fails gracefully and does not crash chat input. | Covered: `voice-input.spec.ts` |
| `VOICE-003` | P1 | Recording state. | Holding or activating the mic shows recording state and prevents conflicting send actions. | Covered/Partial: `voice-input.spec.ts`; add actual transcript insertion when supported |
| `VOICE-004` | P1 | Permission denied. | Microphone permission denial leaves text messaging usable and shows a recoverable state. | Gap |
| `VOICE-005` | P1 | Transcript send. | Recognized speech populates the composer and can be sent exactly once. | Gap |

## Server Manager and Deployment

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `SERVER-001` | P1 | Server manager opens. | Sidebar menu exposes Servers and opens/closes the manager modal. | Covered: `server.spec.ts` |
| `SERVER-002` | P1 | SSH host listing. | Configured SSH hosts show host, user, and deploy affordances; empty config is graceful. | Covered: `server.spec.ts` |
| `SERVER-003` | P1 | Deploy trigger. | Clicking deploy shows a status transition and handles command failure visibly. | Partial: `server.spec.ts`; add failed deploy assertion |
| `SERVER-004` | P0 | Remote command safety. | Server manager cannot execute deployment commands for unauthorized users or arbitrary host strings. | Gap |

## Visual Status Surfaces

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `VIZ-001` | P2 | Pixel World toggle. | Pixel World opens, closes, and persists open state after reload. | Covered: `pixel-world.spec.ts` |
| `VIZ-002` | P2 | Pixel World canvas. | Canvas renders a non-empty scene and does not produce console errors. | Covered/Partial: `pixel-world.spec.ts`; add pixel-level nonblank assertion if rendering regresses |
| `VIZ-003` | P2 | Empty agent state. | Pixel World handles companies with no agents gracefully. | Covered: `pixel-world.spec.ts` |
| `VIZ-004` | P1 | Agent status accuracy. | Visual agent status matches runtime status from console snapshot. | Gap |

## Git and Repository Views

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `GIT-001` | P1 | Git tab visibility. | Group detail panel exposes Git tab where repository context exists. | Covered: `git-graph.spec.ts`, `detail-panel.spec.ts` |
| `GIT-002` | P1 | Graph load. | Git graph endpoint returns data or a graceful error; UI shows loading, branch names, and graph SVG/canvas. | Covered: `git-graph.spec.ts` |
| `GIT-003` | P1 | Non-git workspace. | Non-repository folders show an actionable empty/error state without console failures. | Gap |
| `GIT-004` | P0 | Git path isolation. | Git graph API cannot inspect repositories outside the active company/workspace boundary. | Partial: `apps/web/app/api/git-graph/route.test.ts` covers workspace-scoped repo-path rejection before invoking git for `B-022`; `apps/web/lib/workspace/git-graph-repo-path.test.ts` covers active-workspace binding selection; broaden if additional repository inspection endpoints are added |

## Templates and Documentation

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `TEMPLATE-001` | P1 | Curated template validation. | Agent and team templates expose valid drivers, required inputs, and coordinator metadata. | Covered: `team-templates.test.ts`, `team-template-validation.test.ts` |
| `TEMPLATE-002` | P1 | Template selection. | Create Agent and Create Group render curated templates and produce a reviewable draft. | Covered: `create-agent-template-flow.test.ts`, `create-group-template-flow.test.ts` |
| `TEMPLATE-003` | P1 | Team launch. | Provisioning creates the selected agents and group or returns actionable recovery state. | Covered: `group-provisioning-runner.test.ts` and route tests |
| `DOCS-001` | P2 | Docs navigation. | Important docs pages load with layout/navigation intact. | Covered: `docs.spec.ts` |
| `DOCS-002` | P2 | Docs links. | Docs cross-links do not point at missing routes or stale feature names. | Gap |

## Telemetry

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `TELEM-001` | P1 | Analytics event shape. | Analytics POSTs include event name, timestamp, and expected metadata. | Covered: `telemetry.spec.ts` |
| `TELEM-002` | P0 | Sensitive data exclusion. | Session tokens, secrets, private message contents, attachment names/bytes, and local paths are excluded from telemetry payloads, logs, and persistence. | Covered: `apps/web/lib/api/telemetry-sanitize.test.ts`, `apps/web/app/api/analytics/route.test.ts`, and `services/choruz-api-gateway/src/tests/::telemetry_ingest_redacts_sensitive_payloads_before_persisting` |
| `TELEM-003` | P1 | Analytics outage. | Analytics endpoint failure does not break product workflows. | Covered: `telemetry.spec.ts` |
| `TELEM-004` | P1 | Trace correlation. | Conversation switch and message send traces can be correlated without exposing credentials. | Covered/Partial: `telemetry.spec.ts` |

## UX, Accessibility, and Layout

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `UX-001` | P1 | Sidebar basics. | Sidebar renders user, company selector, actions menu, search, and conversation list. | Covered: `sidebar.spec.ts`, `dashboard.spec.ts` |
| `UX-002` | P1 | Keyboard messaging. | Enter sends, Shift+Enter inserts newline, mention dropdown supports arrows/Tab/Escape. | Covered: `keyboard.spec.ts` |
| `UX-003` | P1 | Responsive layout. | Desktop, tablet, mobile, and wide viewports keep sidebar/chat usable without overlap. | Covered: `responsive.spec.ts` |
| `UX-004` | P1 | Detail panel. | Panel opens/closes, shows correct tabs by conversation type, and persists width. | Covered: `detail-panel.spec.ts` |
| `UX-005` | P2 | Theme. | Theme toggles, persists after reload, and keeps accessible labels/icons. | Covered: `theme.spec.ts` |
| `UX-006` | P2 | Sidebar hover polish. | Hover text and actions are visually clear and do not obscure content. | Manual: `C-001` needs acceptance criterion |
| `UX-007` | P1 | Modal focus and validation. | Create Agent, Group, and Company modals validate required fields, close safely, and trap focus. | Covered: `modals.spec.ts` |
| `UX-008` | P1 | Chat header state. | Header shows selected conversation title, subtitle, avatar, mobile menu, detail toggle, and connection status. | Covered: `chat-header.spec.ts` |

## Observability and API Health

| ID | Priority | Scenario | Expected Behavior | Automation |
| --- | --- | --- | --- | --- |
| `OBS-001` | P0 | API health. | Health endpoint reports service readiness. | Covered/Partial: `api-routes.spec.ts`; add host smoke if needed |
| `OBS-002` | P1 | Console snapshot. | `GET /v1/console` returns principal, companies, agents, conversations, and runtime status. | Covered: `api-routes.spec.ts`, many e2e fixtures |
| `OBS-003` | P1 | Metrics. | Pipeline and gateway metrics expose backlog, latency, and failure indicators. | Gap |
| `OBS-004` | P1 | Dead-letter visibility. | Failed pipeline events can be discovered with reason, event ID, and retry state. | Gap |

## Bug Capture Rule

When a scenario fails:

1. Re-run the focused test once to rule out a transient host issue.
2. Save the exact command, trace, screenshot, or one-off probe path.
3. Add a row to `BUGS.md` if there is no existing row.
4. Include the scenario ID in the bug row, for example:

```text
Repro Artifact: `apps/web/tests/e2e/agent-dm.spec.ts -g "AGENT-004"`; scenario `AGENT-004`
```

5. Keep the bug in `NEEDS_REPRO` if the evidence is only manual or flaky.
6. Move to `TODO` only when another agent can independently reproduce it.

## Recommended Next Test Additions

### Release Blocker Audit

Release-blocker decisions from the 2026-05-16 audit. Use this table for release
planning before treating every P0 scenario below as mandatory automation.

| Scenario | Release Decision | Expected Behavior |
| --- | --- | --- |
| `COMP-008` | Blocker | The company owner can access owned companies but cannot use that exception to leak unrelated workspaces. |
| `AGENT-007` | Blocker | An agent never receives another agent's direct chat history or workspace context. |
| `TEAM-007` | Blocker | Agents only receive messages, files, workspace paths, and prior context they are authorized to see, even during handoffs or artifact review. |
| `CHAT-008` | Blocker | Group conversations show the normal transcript and composer; direct terminal-mode bindings show the PTY surface. |
| `MENTION-002` | Blocker | Mentioning an agent outside the group or workspace does not route a command. |
| `OUTBOX-001` | Blocker | Agent-originated `.choruz/send` group text is delivered to the named group and visible to members. |
| `RT-006` | Blocker | Event backlog drains under normal traffic and dead letters are diagnosable. |
| `CRON-004` | Blocker | A due job inserts a message and triggers the target agent through the normal pipeline. |
| `TELEM-002` | Blocker | Session tokens, secrets, private message contents, and attachment bytes are not sent in analytics. |
| `B-018` | Blocker | Concurrent agent provisioning does not corrupt `.runtime/agent_tokens.json` or return 500. |

Not blockers for this release: `COMP-002`, `COMP-003`, `AGENT-002`,
`AGENT-004`, `AGENT-010`, `AGENT-011`, `TEAM-001`, `TEAM-002`, `CHAT-010`,
`MENTION-008`, `SERVER-004`, `FILE-006`, `GIT-004`, `OBS-001`, `B-016`, and
`B-017`.

These are the highest-value gaps to close next, sorted by scenario priority and
then by current product risk. This is not the full backlog; it is the next
practical slice to automate.

| Rank | Priority | Scenario | Why It Matters | Suggested Artifact |
| --- | --- | --- | --- | --- |
| 1 | P0 | `B-018` | Covered release blocker: parallel provisioning must not corrupt token persistence or return 500 during setup. | `agent.spec.ts` exercises parallel full-route provisioning; `apps/web/lib/agents/agent-tokens.test.ts` stress-tests concurrent persistence and file permissions. |
| 2 | P0 | `COMP-008` | Release blocker: company-owner exceptions are an authorization boundary and must not leak unrelated workspaces. | Covered by `company_workspace_authorization_guards_hold` |
| 3 | P0 | `AGENT-007` | Release blocker: B-005 has deterministic session-isolation coverage; finish remaining real-driver confidence check. | Focused real-driver/manual smoke plus existing Choruz router/pipeline/API gateway regressions |
| 4 | P0 | `TEAM-007` | Release blocker: multi-agent privacy boundaries must hold for messages, files, workspace paths, and prior context. | Partial: API surfaces and persisted-source webhook handoff isolation are covered; add pipeline prior-context and attachment-handoff regression |
| 5 | P0 | `CHAT-008` | Release blocker: group/direct mode is basic product behavior and must not regress. | Strengthen `terminal.spec.ts` or add focused e2e for group composer vs direct PTY |
| 6 | P0 | `MENTION-002` | Release blocker: mention routing must not wake agents outside the group or workspace. | Covered by existing router/provider tests; add UI/API smoke only if mention UI filtering changes |
| 7 | P0 | `OUTBOX-001` | Release blocker: agent-originated `.choruz/send` group text is core to agent workflows. | Reviewed coverage added in `outbox_handler.rs::process_outbox_commands_delivers_group_send_to_named_group`; broader UI outbox coverage remains in `outbox.spec.ts` |
| 8 | P0 | `RT-006` | Release blocker: pipeline backlog and dead-letter visibility are required to diagnose broken agent workflows. | Reviewed router coverage added for valid backlog drain and malformed outbox dead-lettering; full host smoke remains a residual gap |
| 9 | P0 | `CRON-004` | Release blocker: scheduled work is launch scope and must trigger the target agent through the normal pipeline. | Reviewed scheduler coverage added for due announced job inserting a visible message and pending agent command; full executor smoke remains a residual gap |
| 10 | P0 | `TELEM-002` | Release blocker: telemetry must not leak tokens, secrets, private messages, attachment names/bytes, or local paths. | Covered by frontend sanitizer, legacy analytics log, and gateway persistence regressions |
| 11 | P0 | `COMP-002` | Not a release blocker: major cross-company leaks are fixed; broader active-company/file/attachment UI coverage can follow. | Fast-follow e2e/API coverage |
| 12 | P0 | `TEAM-001` / `TEAM-002` | Not a release blocker for cross-platform collaboration; deterministic webhook-agent collaboration is covered and mixed real-driver behavior remains environment-dependent. | `team-collaboration.spec.ts` plus final Claude/Codex real-driver CUA |
| 13 | P0 | `AGENT-010` / `AGENT-011` | Not a release blocker: provider behavior will be manually tested; executor contracts already have deterministic coverage. | Fast-follow API/runtime matrix and webhook integration tests |
| 14 | P1 | `ATTACH-006` / `ATTACH-007` | Fast-follow coverage outside the current release-blocker audit; still valuable for durable attachment hardening. | Extend `apps/web/tests/e2e/attachment.spec.ts` plus API tests |
| 15 | P1 | `OUTBOX-005` | Fast-follow coverage outside the current release-blocker audit; bad agent commands should still become diagnosable errors. | `apps/web/tests/e2e/outbox.spec.ts` or pipeline test |
