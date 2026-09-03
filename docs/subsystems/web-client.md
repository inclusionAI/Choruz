# Web client

The web client is the Next.js 16 / React 19 application in `apps/web` that renders the dashboard, the chat surface, the in-app docs site and the Next.js API routes that front provisioning and filesystem work. A reader can use this page to find which module owns a piece of browser behaviour (message cache, sync stream, telemetry, modals, team templates, pixel world) and where its tests live. Source: [`../../apps/web`](../../apps/web).

## Layout

`lib/` is grouped by domain; a new module goes in the folder whose name matches the concept it serves, and a React hook goes in `hooks/`:

| Folder | Holds |
|---|---|
| `lib/api/` | gateway client, wire types, route auth guard, request origin, telemetry (`choruz-api.ts`, `choruz-types.ts`, `api-auth.ts`, `choruz-trace.ts`, `telemetry-sanitize.ts`, `principals.ts`, `request-origin.ts`) |
| `lib/messages/` | message cache and IndexedDB store, threads and thread unreads, mentions, quotes, conversation flags, sidebar ordering |
| `lib/api/transport.ts` | the two ways out of the browser: `transportFetch` for same-origin `/api/*` calls and `transportSocket` for the gateway sockets; the local transport is the default, a relay transport can be installed for a remote dashboard |
| `lib/remote/relay-session.ts` | The paired browser's Remote Control session: rendezvous socket, transport offer, `device.hello`, AES-GCM envelopes in send and receive order, `RelayStatus`; `RelayLink` is the narrow surface the transport needs. |
| `lib/remote/relay-pairing.ts` | Device side of the committed ECDH pairing handshake (`pairWithHost`) and per-gateway credential storage in `localStorage`. |
| `lib/remote/relay-transport.ts` | `createRelayTransport(link)`: a `ChoruzTransport` that turns fetches into `http.*` frames and gateway sockets into `stream.*` frames (`RelaySocket`), chunked at `CHUNK_BYTES`; the frame contract is `remote_control_executor.rs`. |
| `lib/agents/` | agent provisioning, instruction templates, tokens, AI-manager instructions, harness accounts |
| `lib/drivers/` | driver registry, availability, model catalogue and validation |
| `lib/groups/` | team templates and rendering, group provisioning jobs (contract, runner, store, db, issue display) |
| `lib/channel-tasks/` | board task assignees, creation, reconciliation and shared task helpers |
| `lib/terminal/` | terminal bindings, PTY write buffer, ANSI stripping |
| `lib/remote/` | remote-control hosts, pairing crypto, remote server install |
| `lib/workspace/` | workspace path guard, git-graph repository path resolution |
| `lib/` (root) | cross-cutting helpers only: `utils.ts`, `format-bytes.ts`, `format-chat-time.ts`, `audio-utils.ts`, `avatar.ts` |
| `hooks/` | every `use-*.ts` React hook, including `use-chat-web-socket.ts` |

`components/` follows the same rule, one folder per feature surface:

| Folder | Holds |
|---|---|
| `components/chat/` | `chat-app.tsx` (the dashboard orchestrator), header, input, modals, sidebar, conversation rows, detail panel, message list and bubble, thread panel, channel conversation tabs |
| `components/channel-tasks/` | the board and the create-task modal |
| `components/agents/` | agent config editor, instruction form, skills list, create-agent modal, driver pickers, harness account picker, summary and modal, workspace session import |
| `components/groups/` | create-group and create-company modals, step tabs and setup input fields they share |
| `components/workspace/` | file tree, file editor, path and folder pickers, git graph |
| `components/runtime/` | server, runtime host and remote-control managers, runtime status panel, terminal view |
| `components/ui/` | primitives with no product knowledge: `modal.tsx`, `spinner.tsx`, `avatar.tsx`, `empty-state.tsx`, `resize-handle.tsx`, `theme-provider.tsx` |
| `components/pixel-world/` | the canvas renderer, its game scenes and unit tests |

Styles are plain CSS: `app/globals.css` is an ordered `@import` manifest and each section lives in `app/styles/<section>.css` (theme tokens first, then the chat shell, header, message list, composer, detail panel, modals, responsive rules, and one file per feature surface). A new rule goes in the file for its surface; a new surface gets a file and an `@import` line at the position its rules must win or lose from.

Unit tests sit next to the module they pin (`foo.test.ts` beside `foo.ts`).

## Owns

| Area | Path |
|---|---|
| App Router entry | [`app/page.tsx`](../../apps/web/app/page.tsx) (`LocalEntryPage`, redirects to `/dashboard` when the session cookie is present), [`app/dashboard/page.tsx`](../../apps/web/app/dashboard/page.tsx) (`DashboardPage`, calls `fetchDashboardBootstrap(sessionToken, { limit: 100 })` and maps it with `lib/api/dashboard-snapshot.ts`), [`app/remote/page.tsx`](../../apps/web/app/remote/page.tsx) (`RemotePage`, renders `components/remote/remote-dashboard.tsx`: pairs through the Cloud Gateway, installs the relay transport, fetches the bootstrap through it and mounts the same `ChatApp`), [`app/layout.tsx`](../../apps/web/app/layout.tsx) |
| Auth routes | [`app/auth/logout/route.ts`](../../apps/web/app/auth/logout/route.ts), [`app/auth/session-invalid/route.ts`](../../apps/web/app/auth/session-invalid/route.ts) |
| Export route | [`app/export/[conversationId]/route.ts`](../../apps/web/app/export/[conversationId]/route.ts) (`GET`, wraps `exportConversation`) |
| In-app docs | [`app/docs/layout.tsx`](../../apps/web/app/docs/layout.tsx) (sidebar nav, `docs.css`) and one `page.tsx` per topic under `app/docs/{getting-started,concepts,features,agents,api,operations,troubleshooting,reference}` |
| Next.js API routes | `app/api/agent-config`, `agent-skills`, `agents/provision`, `agents/batch-disable`, `analytics`, `attachments/[id]`, `companies`, `drivers/availability`, `drivers/models`, `filesystem`, `git-graph`, `group-provisioning-jobs` (plus `[jobId]/{run,retry,cancel}`), `harness-accounts` (plus `[id]/probe`, `[id]/login`), `skills/scan` under [`app/api`](../../apps/web/app/api) |
| Route auth guard | [`lib/api/api-auth.ts`](../../apps/web/lib/api/api-auth.ts) `requireAuth` verifies the `choruz_session` cookie against `GET /v1/me` on the gateway |
| Chat shell | [`components/chat/chat-app.tsx`](../../apps/web/components/chat/chat-app.tsx) plus `sidebar.tsx`, `message-list.tsx`, `message-bubble.tsx`, `chat-input.tsx`, `chat-header.tsx`, `chat-modals.tsx`, `detail-panel.tsx`, `terminal-view.tsx`, `thread-panel.tsx` |
| Gateway client | [`lib/api/choruz-api.ts`](../../apps/web/lib/api/choruz-api.ts) (`apiBaseUrl`, `apiJson`, `apiFetch`, `fetchDashboardBootstrap`, `sendMessage`, `fetchThread`, `markThreadViewed`, `fetchChannelTasks`, `patchChannelTask`, …), types in [`lib/api/choruz-types.ts`](../../apps/web/lib/api/choruz-types.ts) |
| Sync stream | [`hooks/use-chat-web-socket.ts`](../../apps/web/hooks/use-chat-web-socket.ts) connects to `/v1/ws/sync`, handles `sync_ready`, `sync_changes`, `sync_acked`, `sync_error`, sends `sync_ack` |
| Message cache | [`lib/messages/messages.ts`](../../apps/web/lib/messages/messages.ts) (pure merge helpers), [`lib/messages/message-db.ts`](../../apps/web/lib/messages/message-db.ts) (Dexie/IndexedDB) |
| Threads | [`lib/messages/threads.ts`](../../apps/web/lib/messages/threads.ts), [`lib/messages/thread-unreads.ts`](../../apps/web/lib/messages/thread-unreads.ts); see [threads.md](threads.md) |
| Hooks delegated from chat-app | [`hooks/use-conversation-flags.ts`](../../apps/web/hooks/use-conversation-flags.ts), [`hooks/use-company-management.ts`](../../apps/web/hooks/use-company-management.ts), [`hooks/use-panel-resize.ts`](../../apps/web/hooks/use-panel-resize.ts), [`hooks/use-message-search.ts`](../../apps/web/hooks/use-message-search.ts), `use-edge-swipe.ts`, `use-thinking-agents.ts`, `use-modal-a11y.ts` |
| Helpers | [`lib/terminal/terminal-bindings.ts`](../../apps/web/lib/terminal/terminal-bindings.ts), [`lib/messages/mentions.ts`](../../apps/web/lib/messages/mentions.ts), [`lib/api/principals.ts`](../../apps/web/lib/api/principals.ts), [`lib/messages/conversation-flags.ts`](../../apps/web/lib/messages/conversation-flags.ts) |
| Telemetry | [`lib/api/choruz-trace.ts`](../../apps/web/lib/api/choruz-trace.ts), [`lib/api/telemetry-sanitize.ts`](../../apps/web/lib/api/telemetry-sanitize.ts), [`app/api/analytics/route.ts`](../../apps/web/app/api/analytics/route.ts) |
| Modal shell and shared fields | [`components/ui/modal.tsx`](../../apps/web/components/ui/modal.tsx) (`Modal`), [`components/groups/setup-input-field.tsx`](../../apps/web/components/groups/setup-input-field.tsx) (`SetupInputField`), [`components/workspace/path-picker.tsx`](../../apps/web/components/workspace/path-picker.tsx) |
| Client plugins | [`plugins/registry.ts`](../../apps/web/plugins/registry.ts) (`resolveClientPluginIds`), [`plugins/client-plugin.ts`](../../apps/web/plugins/client-plugin.ts), one `client.tsx` each under `plugins/{kanban,pixel-world,workspace-git,remote-ssh,remote-control,agent-skills}` |
| Pixel world | [`components/pixel-world`](../../apps/web/components/pixel-world) (`pixel-world.tsx`, `pixel-world-store.ts`, `game/`, `docs/`) behind [`plugins/pixel-world/client.tsx`](../../apps/web/plugins/pixel-world/client.tsx) |
| Team templates | [`lib/groups/team-templates.ts`](../../apps/web/lib/groups/team-templates.ts), [`lib/groups/team-template-renderer.ts`](../../apps/web/lib/groups/team-template-renderer.ts), [`lib/groups/team-template-validation.ts`](../../apps/web/lib/groups/team-template-validation.ts), `create-agent-template-flow.ts`, `create-group-template-flow.ts`, `group-provisioning-{runner,store,contract}.ts`, and the instruction fragments in [`agent-templates/`](../../agent-templates) composed by [`lib/agents/agent-instruction-template.ts`](../../apps/web/lib/agents/agent-instruction-template.ts) |
| Build and test config | [`next.config.ts`](../../apps/web/next.config.ts), [`vitest.config.ts`](../../apps/web/vitest.config.ts), [`playwright.config.ts`](../../apps/web/playwright.config.ts), [`infra/host/web_e2e.sh`](../../infra/host/web_e2e.sh), [`scripts/prepare-next-types.mjs`](../../apps/web/scripts/prepare-next-types.mjs) |

## Data

`ChatMessage` ([`lib/api/choruz-types.ts`](../../apps/web/lib/api/choruz-types.ts)) is the per-message row the client keeps: `id`, `conversation_id`, `sender_id`, `content`, `content_type`, `metadata`, `server_seq`, `idempotency_key`, `created_at`. `MessagesByConv` in `lib/messages/messages.ts` is `Record<string, ChatMessage[]>`, the in-memory cache keyed by conversation.

`OPTIMISTIC_SERVER_SEQ` (`Number.MAX_SAFE_INTEGER`, defined once in `lib/messages/messages.ts`) marks a locally inserted row that the server has not confirmed; `upsertConfirmedMessage`, `mergeFetchedMessages`, `appendIncrementalMessages`, `mergePreviewIntoMessages`, `messagesMissingFromPrevious` and `maxCachedSeq` all key on it.

`MessageDatabase` in `lib/messages/message-db.ts` is the Dexie database `choruz_messages` with two tables: `messages` (primary key `[conversation_id+server_seq]`, index `conversation_id`) and `syncState` (primary key `&principal_id`, rows of `DashboardSyncState { principal_id, device_id, ack_cursor }`). Exports: `persistMessages`, `loadConversationMessages`, `loadAllCachedMessages`, `maxPersistedSeq`, `loadDashboardSyncState`, `persistDashboardSyncCursor`, `resetMessageDb`.

`DashboardBootstrap` and `DashboardSyncChange` (`lib/api/choruz-types.ts`) are the bootstrap page and the sync-feed change rows; `chat-app.tsx` applies changes by `event_type` / `entity_type` (message rows, `conversation.deleted`, pin/archive/hidden flags, `conversation.read_state_changed`, `thread.read_state_changed`, `channel_task`) and falls back to a bootstrap refresh for anything else.

`HostPluginManifest` rows from `bootstrap.plugins` are matched against `CLIENT_PLUGINS` in `plugins/registry.ts`; `chat-app.tsx` derives `kanbanEnabled`, `pixelWorldEnabled`, `workspaceGitEnabled`, `remoteSshEnabled`, `remoteControlEnabled` and `agentSkillsEnabled` from the resulting id set.

`RoleTemplate`, `GroupTemplate`, `SetupInput` (`type: "text" | "textarea" | "path" | "select"`), `OutputContract` and the constants `ROLE_TEMPLATES`, `GROUP_TEMPLATES`, `BOARD_TASKS_CREATED_SECTION` live in `lib/groups/team-templates.ts`; `renderRoleInstructions` and `renderGroupKickoff` in `lib/groups/team-template-renderer.ts` turn them into agent instructions and kickoff text.

`TraceEntry` and `Span` in `lib/api/choruz-trace.ts` are the telemetry records; `trace.start(name, data)` returns a span, `trace.event(name, data)` is one-shot, `traceRing()` exposes the in-memory ring, and every payload passes through `sanitizeTelemetryData` before leaving the browser.

`PixelWorldState` in `components/pixel-world/pixel-world-store.ts` is a zustand store (`usePixelWorldStore`) holding `PixelAgentState`, `HouseInfo`, `PlayerState` and `WalkabilityMask`; `emitPixelWorldEvent` is the bridge chat-app uses to animate agents.

## Entry points

- Browser: `/` → `/dashboard`; the server component renders `ChatApp` with the bootstrap snapshot, then the client opens `/v1/ws/sync?device_id=…&cursor=…` through `useChatWebSocket` and re-fetches `/v1/unreads` on demand.
- Gateway traffic: `next.config.ts` rewrites `/api/v1/:path*` to the gateway resolved from `CHORUZ_API_BASE_URL`, `CHORUZ_API_URL` or `CHORUZ_API_PORT` (default `http://127.0.0.1:3000`); `apiBaseUrl()` in `lib/api/choruz-api.ts` uses the same precedence server-side, and `NEXT_PUBLIC_CHORUZ_API_PORT` is exposed to the browser.
- Telemetry: `choruz-trace.ts` POSTs entries to `/api/v1/telemetry` (the rewrite forwards to the gateway); `POST /api/analytics` is log-only after `sanitizeTelemetryValue`.
- Next.js API routes call `requireAuth` first, then use `CHORUZ_RUNTIME_DIR`, `CHORUZ_GIT_REPO_PATH`, `CHORUZ_INTERNAL_PROVISION_TOKEN`, `CHORUZ_DATABASE_URL` / `CHORUZ_PG_*` and the `CHORUZ_{CLAUDE,CODEX,PI,GROK,OPENCODE}_BINARY` variables for provisioning and filesystem work.
- Docs: `/docs` and the nested topic pages are static App Router pages with no gateway dependency.
- Tests: `pnpm --dir apps/web test` (vitest), `pnpm --dir apps/web e2e` (Playwright), `bash infra/host/web_e2e.sh [spec…]` (full stack), `pnpm --dir apps/web check` (`prepare-next-types.mjs` then `tsc --noEmit`).

## Invariants

- A sync page is applied to state and persisted before its cursor is acknowledged (`useChatWebSocket` sends `sync_ack` with `frame.next_cursor` after `onChanges` resolves), so a crash during apply replays the page; pinned by `tests/e2e/websocket.spec.ts` ("converges on one message in two tabs", "deduplicates optimistic messages with sync confirmations").
- Optimistic rows never reach IndexedDB: `persistMessages` filters `server_seq < OPTIMISTIC_SERVER_SEQ`; pinned by `lib/messages/message-db.test.ts` and `lib/messages/messages.test.ts`.
- `mentionedAgentIds` mirrors the router's `@all` / `@name` rules so the thinking indicator matches which agents wake; pinned by `lib/messages/mentions.test.ts`.
- A terminal binding stays mounted for every open tab (`openTerminalBindings`), so switching tabs never drops the PTY WebSocket; pinned by `lib/terminal/terminal-bindings.test.ts`.
- Every Next.js API route that acts on a principal goes through `requireAuth`, which round-trips to `GET /v1/me` rather than trusting `decodeSessionClaims`; pinned by the `route.test.ts` files beside each route.
- A client plugin renders only when the host manifest satisfies `requiredHostCapabilities` (`hostSupportsClientPlugin`); pinned by `plugins/registry.test.ts` and `tests/e2e/plugins.spec.ts`.
- Telemetry payloads are redacted before send: `sanitizeTelemetryData` / `sanitizeTelemetryValue`; pinned by `lib/api/telemetry-sanitize.test.ts` and `tests/e2e/telemetry.spec.ts` ("should not include session token in analytics payload").
- Role templates never require a prose "Assignments" section; coordinator roles require `BOARD_TASKS_CREATED_SECTION`; pinned by `lib/groups/team-templates.test.ts` and `lib/groups/team-template-renderer.test.ts`.

## Failure modes

- IndexedDB unavailable (quota, private mode, schema upgrade): every `message-db.ts` operation catches and emits `trace.event("indexeddb_fallback", { op, error })`; the chat path re-fetches over HTTP.
- Sync WebSocket drop: `useChatWebSocket` reports `status: "reconnecting"` and retries with backoff from `RECONNECT_BASE_MS` (500 ms) to `RECONNECT_MAX_MS` (16 s), resuming from the persisted `ack_cursor`.
- Bootstrap or bindings fetch failure on `/dashboard`: `DashboardPage` logs `[dashboard] fetch failed source=…` and renders with empty companies and bindings instead of failing the page.
- Gateway unreachable from `requireAuth`: the route answers `503 Auth service unavailable` (3 s timeout) rather than `401`.
- Unknown sync change types trigger a full bootstrap refresh (`refreshBootstrap = true` in `chat-app.tsx`), visible as an extra `GET /v1/bootstrap`.
- Telemetry endpoint failures are swallowed inside `choruz-trace.ts`; `tests/e2e/telemetry.spec.ts` pins "should not crash when analytics endpoint is unavailable".

## Tests

- Unit (vitest, `environment: "node"`, include `lib/**/*.test.ts`, `components/**/*.test.ts`, `app/**/*.test.ts`, `plugins/**/*.test.ts`): [`lib/messages/messages.test.ts`](../../apps/web/lib/messages/messages.test.ts), [`lib/messages/messages.integration.test.ts`](../../apps/web/lib/messages/messages.integration.test.ts), [`lib/messages/message-db.test.ts`](../../apps/web/lib/messages/message-db.test.ts) (uses `fake-indexeddb`), [`lib/api/choruz-api.test.ts`](../../apps/web/lib/api/choruz-api.test.ts), `choruz-api-fetch.test.ts`, `choruz-api-runtime.test.ts`, [`lib/api/choruz-trace.test.ts`](../../apps/web/lib/api/choruz-trace.test.ts), `telemetry-sanitize.test.ts`, `mentions.test.ts`, `terminal-bindings.test.ts`, `conversation-flags.test.ts`, `sidebar-conversations.test.ts`, `team-templates.test.ts`, `team-template-renderer.test.ts`, `team-template-validation.test.ts`, `create-agent-template-flow.test.ts`, `create-group-template-flow.test.ts`, `group-provisioning-*.test.ts`, `pixel-world-logic.test.ts`, `pixel-world-pathfinding.test.ts`, `pixel-animations.test.ts`, `pixel-houses.test.ts`, `pixel-recolorer.test.ts`, `pixel-tiles.test.ts`, [`components/chat/message-list.test.ts`](../../apps/web/components/chat/message-list.test.ts), `components/chat/message-bubble.test.ts`, [`plugins/registry.test.ts`](../../apps/web/plugins/registry.test.ts), `plugins/server-plugin.test.ts`, and the `route.test.ts` files under `app/api/*` and `app/auth/session-invalid`.
- E2E (Playwright, `testDir: ./tests`, project `chromium`, plus `chromium-reduced-motion` when `CHORUZ_E2E_EXTENDED=1`): [`tests/e2e/app-smoke.spec.ts`](../../apps/web/tests/e2e/app-smoke.spec.ts) (the default spec for `web_e2e.sh`), [`tests/e2e/dashboard.spec.ts`](../../apps/web/tests/e2e/dashboard.spec.ts), `messaging.spec.ts`, `websocket.spec.ts`, `indexeddb.spec.ts`, `telemetry.spec.ts`, `docs.spec.ts`, `api-routes.spec.ts`, `plugins.spec.ts`, `modals.spec.ts`, `sidebar.spec.ts`, `search.spec.ts`, `terminal.spec.ts`, `pixel-world.spec.ts`, `team-collaboration.spec.ts`, `user-journeys.spec.ts`, `responsive.spec.ts`, `theme.spec.ts`, `keyboard.spec.ts`, plus `tests/e2e/message-dedup.spec.ts`, `tests/e2e/outbox-reply.spec.ts`, `tests/pixel-world-*.spec.ts`, `tests/e2e/quotes.spec.ts`, `tests/e2e/threads.spec.ts` and the the `tests/e2e/sweep-*.spec.ts` sweeps.
- Fixtures: [`tests/fixtures/auth.ts`](../../apps/web/tests/fixtures/auth.ts) (`API_BASE`, `WEB_BASE`, `CREDENTIALS` from `CHORUZ_OPERATOR_USER` / `CHORUZ_OPERATOR_PASSWORD`) and [`tests/fixtures/api.ts`](../../apps/web/tests/fixtures/api.ts).
- Harness: `infra/host/web_e2e.sh` starts Postgres via `infra/host/start.sh`, runs `migrate.sh reset` and `up`, builds and starts `choruz-api-gateway` and `choruz-pipeline`, starts `pnpm dev` on `CHORUZ_WEB_PORT` (default 3100), then runs `pnpm e2e "$@"`; with no arguments and without `CHORUZ_WEB_E2E_FULL=1` it runs only `tests/e2e/app-smoke.spec.ts`. CI (`.github/workflows/ci.yml`) runs `vitest related` on changed files and sharded `web_e2e.sh` invocations.

## Related

- [sync-feed.md](sync-feed.md) — the `/v1/ws/sync` contract this client consumes.
- [api-gateway.md](api-gateway.md) — the `/v1` routes behind `apiBaseUrl()`.
- [threads.md](threads.md) — `lib/messages/threads.ts`, `lib/messages/thread-unreads.ts`, `components/chat/thread-panel.tsx`.
- [channel-tasks.md](channel-tasks.md) — the kanban plugin, board components and template receipts.
- [agent-protocol.md](agent-protocol.md) — the `agent-templates/` fragments the web provisioning path composes.
- Agent Notes: [Board tasks created receipt](../../.agents/notes/implemented/feature/2026-08-18-board-tasks-created-receipt.md).
