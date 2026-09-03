# Bridge

The bridge is a standalone Node service that lets people in Slack and Telegram talk to Choruz Agents from their native apps: it forwards platform messages into a Choruz group conversation as the logged-in operator, and receives signed `message.created` webhooks from the API gateway to push Agent replies back to each mapped channel. A reader can use this page to find the config file shape, the table that maps channels to conversations, the webhook signature the gateway sends, the exact response codes the bridge returns, and how to build and test it. Source: [`services/choruz-bridge`](../../services/choruz-bridge/src/index.ts).

## Owns

| Area | Path |
|---|---|
| Process entry and startup order | [`services/choruz-bridge/src/index.ts`](../../services/choruz-bridge/src/index.ts) |
| YAML config loader | [`src/config.ts`](../../services/choruz-bridge/src/config.ts) (`BridgeConfig`, `loadConfig`) |
| Choruz HTTP client | [`src/choruz-client.ts`](../../services/choruz-bridge/src/choruz-client.ts) (`ChoruzClient`) |
| Channel ↔ conversation mapping | [`src/mapping-store.ts`](../../services/choruz-bridge/src/mapping-store.ts) (`MappingStore`, `ChannelMapping`) |
| Inbound webhook HTTP server | [`src/webhook-server.ts`](../../services/choruz-bridge/src/webhook-server.ts) (`WebhookServer`, `WebhookReplayCache`, `isValidChoruzWebhookSignature`) |
| Mention conversion | [`src/mention.ts`](../../services/choruz-bridge/src/mention.ts) (`slackMentionToChoruz`, `telegramMentionToChoruz`, `choruzMentionToPlatform`) |
| Slack adapter (Socket Mode via `@slack/bolt`) | [`src/adapters/slack.ts`](../../services/choruz-bridge/src/adapters/slack.ts) (`SlackAdapter`) |
| Telegram adapter (long polling via `grammy`) | [`src/adapters/telegram.ts`](../../services/choruz-bridge/src/adapters/telegram.ts) (`TelegramAdapter`) |
| Config template and setup guide | [`choruz-bridge.example.yaml`](../../services/choruz-bridge/choruz-bridge.example.yaml), [`README.md`](../../services/choruz-bridge/README.md) |
| Table `bridge_channel_mappings` | [`migrations/0011_bridge_channel_mappings.sql`](../../migrations/0011_bridge_channel_mappings.sql), column rename in [`V019__choruz_database_cutover.sql`](../../migrations/V019__choruz_database_cutover.sql) |
| Endpoints served | `GET /health`, `POST /webhook/choruz` on `webhook.port` (default 3030, bound to `0.0.0.0`) |

The bridge depends on, but does not own, the gateway's event webhook machinery: table `event_webhook` with `webhook_secret` ([`0020_event_webhook_secret.sql`](../../migrations/0020_event_webhook_secret.sql)), route `POST /v1/principals/{principal_id}/event-webhook` ([`handlers_events.rs`](../../services/choruz-api-gateway/src/handlers_events.rs)), and the signer and flusher in [`webhook.rs`](../../services/choruz-api-gateway/src/webhook.rs). The `webhook_agent` driver type ([`0021_driver_type_webhook_agent.sql`](../../migrations/0021_driver_type_webhook_agent.sql)) shares that delivery path for HTTP-hosted Agents; the bridge itself logs in as a human operator and never holds a `webhook_agent` binding.

## Data

`BridgeConfig` ([`config.ts`](../../services/choruz-bridge/src/config.ts)): `choruz { api_url, username, password }`, optional `slack { bot_token, app_token }`, optional `telegram { bot_token }`, `webhook { port, secret }`, `database { connection_string }`. The file path is `CHORUZ_BRIDGE_CONFIG`, default `choruz-bridge.yaml` resolved from the working directory; omitting `slack` or `telegram` disables that adapter.

Table `bridge_channel_mappings (platform, platform_channel_id, choruz_conversation_id, platform_channel_name, created_at)` with primary key `(platform, platform_channel_id)` and index `idx_bridge_mappings_choruz_conv` on `choruz_conversation_id`; `platform` is `'slack'` or `'telegram'`, `platform_channel_id` a Slack channel id or Telegram `chat.id`. The 0011 migration creates the column as `echat_conversation_id`; V019 renames it to `choruz_conversation_id`, which is the name `MappingStore` queries.

`ChannelMapping { platform, platform_channel_id, platform_channel_name }` is what `findByChoruzConversation` returns for fan-out; `getOrCreate(platform, channelId, channelName, createFn)` creates the Choruz group through `createFn` on first contact and inserts with `ON CONFLICT (platform, platform_channel_id) DO NOTHING`.

Platform → Choruz messages are `POST /v1/messages` with `actor_id` (the bridge principal), `conversation_id`, `content`, `content_type: 'text/plain'`, `idempotency_key` `bridge-<ms>-<random>`, and `metadata { bridge_platform, bridge_sender, bridge_user_id }`. New groups are `POST /v1/groups` with `actor_id`, `name` (`slack-<channel name>` or `tg-<chat title>`), `member_ids: [bridgePrincipalId]`.

Choruz → bridge deliveries are the `ChoruzWebhookEnvelope { event_id, event_type, payload { conversation_id, sender?.name, sender_id?, content, metadata? } }` body with headers `x-choruz-event-id`, `x-choruz-timestamp` (unix seconds) and `x-choruz-signature` = `sha256=` + hex HMAC-SHA256 over `timestamp + "." + raw body`, produced by `sign_webhook` in `webhook.rs` and checked by `isValidChoruzWebhookSignature`. `WEBHOOK_REPLAY_WINDOW_SECONDS = 300` bounds clock skew and replay; `MAX_RECENT_WEBHOOK_EVENT_IDS = 10_000` bounds the replay cache.

Webhook registration is `POST /v1/principals/{principal_id}/event-webhook` with `SetEventWebhookRequest { actor_id, url, event_types, secret? }`; the bridge sends `url: http://127.0.0.1:<webhook.port>/webhook/choruz`, `event_types: ['message.created']` and its own `webhook.secret`. The gateway answers `EventWebhookConfig { principal_id, url, event_types, cursor, updated_at, webhook_secret }` and upserts `event_webhook`.

Mentions: `slackMentionToChoruz` rewrites `<@U…>` to `@<agent-name>` using the adapter's `agentNameMap` and leaves unknown ids untouched; `telegramMentionToChoruz` strips the bot's own `@username`; `choruzMentionToPlatform` returns the text unchanged.

## Entry points

Start: `pnpm --dir services/choruz-bridge start` runs `node dist/index.js`; `pnpm dev` runs `tsx watch src/index.ts`. `index.ts` executes in order: `loadConfig()`, `ChoruzClient.login()` (`POST /v1/auth/local/login`, caching `session_token` and `principal.id`), `new MappingStore(connection_string)` (a `pg.Pool`), `SlackAdapter.start()` and `TelegramAdapter.start()` when configured, `WebhookServer.start()`, then `setEventWebhook`. SIGINT and SIGTERM stop the webhook server, adapters and pool, then `process.exit(0)`.

Slack inbound: `App({ token: bot_token, appToken: app_token, socketMode: true })`; `app.message` ignores messages with `bot_id` or without `text`, resolves the mapping (fetching `conversations.info` for the group name), converts mentions, and calls `sendMessage`. Slash commands: `/new-agent <name> <driver>` → `provisionAgent`, `/new-group <name>` → `createGroup` + `createMapping` for the invoking channel.

Telegram inbound: `new Bot(bot_token)` with `bot.start` long polling; `message:text` ignores `from.is_bot`, maps the chat (`tg-<title>`), strips the bot mention, and calls `sendMessage`. Commands: `/new_agent <name> <driver> [instructions]` → `provisionAgent`, `/new_group <name>` → `createGroup` + `createMapping` for the chat.

Choruz outbound: the gateway's `flush_webhooks` posts each undelivered `outbox_event` for a principal to its `event_webhook.url`, in `delivery_seq` order, and advances `cursor` only on a 2xx. The bridge's `POST /webhook/choruz` verifies headers and signature, parses the body, rejects a mismatched `event_id`, drops replays, keeps only `event_type === 'message.created'`, skips payloads whose `metadata.bridge_platform` is set, looks up `findByChoruzConversation(conversation_id)`, and calls `pushToSlack` (`chat.postMessage` with `*<sender>*: <content>`) or `pushToTelegram` (`sendMessage` in MarkdownV2 with a plain-text fallback) per mapped channel.

Agent provisioning from slash commands goes through `ChoruzClient.provisionAgent`, which posts `{ name, driver_type, instructions }` to `/api/agents/provision` relative to `choruz.api_url`; that path is the Next.js route [`apps/web/app/api/agents/provision/route.ts`](../../apps/web/app/api/agents/provision/route.ts), not an API gateway route.

Build: `pnpm --dir services/choruz-bridge build` runs `tsc` with `rootDir: src`, `outDir: dist`, `module: NodeNext`, excluding `src/**/*.test.ts`; `services/choruz-bridge/dist/` is gitignored. Test: `pnpm --dir services/choruz-bridge test` runs `vitest run`. The package is a pnpm workspace member ([`pnpm-workspace.yaml`](../../pnpm-workspace.yaml)).

## Invariants

- Signatures are verified over the exact raw body: the Fastify content-type parser keeps `application/json` as a string, and `isValidChoruzWebhookSignature` compares with `timingSafeEqual`; pinned by `accepts only the HMAC for the exact raw payload` in `webhook-server.test.ts` and, on the sending side, by `sign_webhook_produces_sha256_prefix`, `deterministic_for_same_input` and `signature_binds_timestamp` in `webhook.rs`.
- The `x-choruz-event-id` header must equal the body's `event_id` (400 otherwise) and each `event_id` is delivered at most once within the replay window; pinned by `delivers each signed event ID only once`.
- The gateway's webhook cursor is a contiguous delivered prefix: a failed delivery blocks later events for that principal without stalling other principals, and delivery retries on the next flush; pinned by `failed_event_blocks_its_principal_without_stalling_other_webhooks` in `webhook.rs` and `webhook_deliveries_retry_until_success` in [`tests.rs`](../../services/choruz-api-gateway/src/tests/). At boot `choruz-api-gateway` reloads `event_webhook` rows into memory (`inject_event_webhook`), so registrations survive a gateway restart.
- No echo loops: every bridge-originated message carries `metadata.bridge_platform`, and `POST /webhook/choruz` answers `{ status: 'ignored', reason: 'bridge_originated' }` for it; Slack `bot_id` and Telegram `is_bot` messages are never forwarded.
- One Choruz conversation per `(platform, platform_channel_id)`: `createMapping` uses `ON CONFLICT DO NOTHING` and `getOrCreate` reads before it creates.
- Config is validated before any network call: missing `choruz.api_url|username|password`, `webhook.secret` or `database.connection_string` ends the process with exit code 1 from `loadConfig`.
- A 401 from Choruz triggers exactly one re-login and retry inside `ChoruzClient.request`.
- `telegramMentionToChoruz` with an empty bot username returns the text unchanged; pinned by `leaves text unchanged before Telegram has loaded bot metadata` in `mention.test.ts`.

## Failure modes

- Unreadable config file: `Failed to read config file: <path>` and exit 1. A failed operator login rejects the top-level `await choruz.login()` in `index.ts`, so the process exits before any adapter starts.
- Webhook registration failure logs `[choruz-bridge] Failed to register webhook (endpoint may not exist yet)` and the bridge keeps running; platform → Choruz continues, Choruz → platform never arrives until the next successful start.
- `POST /webhook/choruz` returns 401 `invalid webhook signature` for a missing or bad header, stale timestamp or wrong secret, 400 `invalid payload` for unparseable JSON or a mismatched `event_id`, 400 `missing conversation_id or content`, and 200 with `status: 'ignored'` and `reason` `duplicate_event`, `event_type`, `bridge_originated` or `no_mapping`. On the gateway side a non-2xx leaves the cursor in place and the event is retried on the next flush.
- A platform push that throws is logged as `[webhook] Failed to push to <platform>/<channel>` and reported as `ok: false` inside a 200 `status: 'delivered'` response; because the response is 2xx the gateway advances the cursor and does not redeliver, so a Slack or Telegram outage loses that message on the platform side.
- A mapped channel whose adapter is not configured logs `No adapter for platform "<platform>"` and is reported `ok: false`.
- Slash commands fail with `Failed to provision agent: <message>` or `Failed to create group: <message>` when `provisionAgent` or `createGroup` throws; `provisionAgent` needs `choruz.api_url` to reach a host that serves `/api/agents/provision`.
- PostgreSQL errors from `MappingStore` propagate to the adapter handlers and are logged as `[slack] Failed to forward message to Choruz` or `[telegram] Failed to forward message to Choruz`; the platform message is dropped, not queued.

## Tests

- Bridge unit and integration tests (vitest): [`src/mention.test.ts`](../../services/choruz-bridge/src/mention.test.ts), [`src/webhook-server.test.ts`](../../services/choruz-bridge/src/webhook-server.test.ts) (signature check and injected `WebhookServer` round trip with a fake `MappingStore` and `SlackAdapter`).
- Gateway-side webhook tests: `sign_webhook_produces_sha256_prefix`, `empty_secret_is_rejected`, `deterministic_for_same_input`, `signature_binds_timestamp`, `failed_event_blocks_its_principal_without_stalling_other_webhooks` in [`services/choruz-api-gateway/src/webhook.rs`](../../services/choruz-api-gateway/src/webhook.rs); `webhook_deliveries_retry_until_success` in [`services/choruz-api-gateway/src/tests/observability.rs`](../../services/choruz-api-gateway/src/tests/observability.rs) asserts the `x-choruz-timestamp` and `x-choruz-signature` headers.
- `webhook_agent` delivery path: `webhook_agent_binding_skips_cli_and_succeeds_empty` in [`services/choruz-pipeline/src/executor.rs`](../../services/choruz-pipeline/src/executor.rs) and `drains_via_watcher(WebhookAgent)` in [`outbox_watcher.rs`](../../services/choruz-pipeline/src/outbox_watcher.rs).
- CI ([`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)): the "Bridge build" step runs `pnpm --dir services/choruz-bridge build` when `services/choruz-bridge/**` or `pnpm-lock.yaml` changes; the vitest suite is not run in CI.

## Related

- [api-gateway.md](api-gateway.md) owns `POST /v1/auth/local/login`, `/v1/messages`, `/v1/groups` and the event-webhook route and flusher the bridge relies on.
- [message-pipeline.md](message-pipeline.md) owns the `outbox_event` rows that become webhook deliveries and the `webhook_agent` executor branch.
- [store.md](store.md) owns the migrations directory, including `0011`, `0020`, `0021` and the `V019` column rename.
- [agent-protocol.md](agent-protocol.md) defines the `@agent-name` mention format the bridge converts to and from.
- [web-client.md](web-client.md) hosts the `/api/agents/provision` route the slash commands call.
- Agent Notes: [OpenAPI as the one external contract](../../.agents/notes/implemented/architecture/2026-09-03-openapi-single-contract.md) is the decision behind the stable `/v1` and signed-webhook contracts the bridge consumes; no Agent Note records the bridge design itself.
