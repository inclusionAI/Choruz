# choruz-bridge

Bridge service that connects Choruz to Slack and Telegram. Users on these platforms can chat with Choruz agents through their native apps.

## Architecture

```text
Slack (Socket Mode)  ──┐
                       ├──> choruz-bridge ──> Choruz API (gateway)
Telegram (long poll) ──┘         ^
                                 |
                        Choruz webhook push
                        (POST /webhook/choruz)
```

- **Slack**: Uses Socket Mode via `@slack/bolt`. No public URL needed.
- **Telegram**: Uses long polling via `grammY`. No public URL needed.
- **Choruz -> bridge**: Choruz pushes events via HTTP webhook to the bridge's Fastify server.
- **Channel mapping**: PostgreSQL table `bridge_channel_mappings` maps platform channels to Choruz conversations.

## Prerequisites

- Node.js >= 18
- PostgreSQL with the Choruz database
- Choruz API gateway running (default: `http://localhost:3000`)
- (Optional) Slack Bot Token + App Token with Socket Mode enabled
- (Optional) Telegram Bot Token from @BotFather

## Setup

### 1. Run the database migration

```bash
psql -h 127.0.0.1 -U "$USER" -d choruz -f migrations/0011_bridge_channel_mappings.sql
```

### 2. Create the config file

```bash
cp services/choruz-bridge/choruz-bridge.example.yaml choruz-bridge.yaml
```

Edit `choruz-bridge.yaml` with your credentials:

```yaml
choruz:
  api_url: "http://localhost:3000"
  username: "operator"
  password: "choruz-local"

# Omit the slack section to disable the Slack adapter
slack:
  bot_token: "xoxb-..."
  app_token: "xapp-..."

# Omit the telegram section to disable the Telegram adapter
telegram:
  bot_token: "123456:ABC-..."

webhook:
  port: 3030
  secret: "replace-with-a-random-webhook-secret"

database:
  connection_string: "postgres://<database-user>@127.0.0.1:5432/choruz"
```

### 3. Install and build

```bash
cd services/choruz-bridge
pnpm install
pnpm build
```

### 4. Start the bridge

```bash
# Development (with auto-reload)
pnpm dev

# Production
pnpm start
```

You can override the config file path with:

```bash
CHORUZ_BRIDGE_CONFIG=/path/to/config.yaml pnpm start
```

## Slack Setup

1. Create a Slack App at https://api.slack.com/apps
2. Enable **Socket Mode** under Settings > Socket Mode
3. Generate an **App-Level Token** with `connections:write` scope -> this is your `app_token`
4. Under OAuth & Permissions, add these Bot Token Scopes:
   - `chat:write` (send messages)
   - `channels:history` (read messages in public channels)
   - `groups:history` (read messages in private channels)
   - `channels:read` (get channel info)
   - `commands` (slash commands)
5. Install the app to your workspace -> copy the **Bot User OAuth Token** as `bot_token`
6. (Optional) Register slash commands under Slash Commands:
   - `/new-agent` - Provision a new Choruz agent
   - `/new-group` - Create a new Choruz group and link it to the current channel
7. Invite the bot to the channels you want bridged

## Telegram Setup

1. Message @BotFather on Telegram to create a new bot
2. Copy the bot token as `bot_token`
3. Add the bot to your Telegram group(s)
4. Send `/new_group <name>` to link a Telegram group to a Choruz conversation
5. Available commands:
   - `/new_agent <name> <driver>` - Provision a new Choruz agent
   - `/new_group <name>` - Create a new Choruz group linked to this chat

## How It Works

### Platform -> Choruz

1. User sends a message in Slack/Telegram
2. Bridge receives it via Socket Mode / long polling
3. Bridge looks up (or auto-creates) the Choruz conversation for that channel
4. Bridge forwards the message to Choruz via `POST /v1/messages`
5. Message metadata includes `bridge_platform`, `bridge_sender`, `bridge_user_id`

### Choruz -> Platform

1. A Choruz agent replies in a bridged conversation
2. Choruz pushes a webhook event to `POST /webhook/choruz`
3. Bridge looks up which platform channels are mapped to that conversation
4. Bridge sends the message to each mapped channel via the platform API
5. Messages originating from the bridge (detected by `bridge_platform` metadata) are skipped to prevent loops

### Channel Mapping

On first message from a new platform channel, the bridge automatically:
1. Creates a Choruz group conversation (named `slack-{channel}` or `tg-{chat_title}`)
2. Records the mapping in `bridge_channel_mappings`

You can also explicitly link channels using the `/new-group` command.

## Configuration Reference

| Key | Required | Description |
|-----|----------|-------------|
| `choruz.api_url` | Yes | Choruz API gateway URL |
| `choruz.username` | Yes | Login username |
| `choruz.password` | Yes | Login password |
| `slack.bot_token` | No | Slack Bot User OAuth Token (`xoxb-...`) |
| `slack.app_token` | No | Slack App-Level Token (`xapp-...`) |
| `telegram.bot_token` | No | Telegram Bot API token |
| `webhook.port` | Yes | Port for the webhook HTTP server (default: 3030) |
| `webhook.secret` | Yes | Shared secret used to authenticate Choruz webhook deliveries |
| `database.connection_string` | Yes | PostgreSQL connection string |
