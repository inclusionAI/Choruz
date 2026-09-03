import { loadConfig } from './config.js';
import { ChoruzClient } from './choruz-client.js';
import { MappingStore } from './mapping-store.js';
import { SlackAdapter } from './adapters/slack.js';
import { TelegramAdapter } from './adapters/telegram.js';
import { WebhookServer } from './webhook-server.js';

const config = loadConfig();

console.log('[choruz-bridge] Config loaded');
console.log(`[choruz-bridge] Choruz API: ${config.choruz.api_url}`);
console.log(`[choruz-bridge] Webhook port: ${config.webhook.port}`);
if (config.slack) console.log('[choruz-bridge] Slack adapter: enabled');
if (config.telegram) console.log('[choruz-bridge] Telegram adapter: enabled');

// ── initialize core services ──────────────────────────────────────────

const choruz = new ChoruzClient(config.choruz);
const loginResult = await choruz.login();
console.log(`[choruz-bridge] Logged in as principal ${loginResult.principal_id}`);

const mappings = new MappingStore(config.database.connection_string);

// ── adapters ──────────────────────────────────────────────────────────

let slackAdapter: SlackAdapter | undefined;
let telegramAdapter: TelegramAdapter | undefined;

if (config.slack) {
  slackAdapter = new SlackAdapter({
    botToken: config.slack.bot_token,
    appToken: config.slack.app_token,
    choruz,
    mappings,
    bridgePrincipalId: loginResult.principal_id,
  });
  await slackAdapter.start();
}

if (config.telegram) {
  telegramAdapter = new TelegramAdapter({
    botToken: config.telegram.bot_token,
    choruz,
    mappings,
    bridgePrincipalId: loginResult.principal_id,
  });
  await telegramAdapter.start();
}

// ── webhook server ────────────────────────────────────────────────────

const webhookServer = new WebhookServer({
  port: config.webhook.port,
  secret: config.webhook.secret,
  mappings,
  slack: slackAdapter,
  telegram: telegramAdapter,
});
await webhookServer.start();

// Register webhook URL with Choruz so it pushes events to us.
try {
  await choruz.setEventWebhook({
    actorId: loginResult.principal_id,
    url: `http://127.0.0.1:${config.webhook.port}/webhook/choruz`,
    secret: config.webhook.secret,
  });
  console.log('[choruz-bridge] Registered event webhook with Choruz');
} catch (err) {
  console.warn(
    '[choruz-bridge] Failed to register webhook (endpoint may not exist yet):',
    err,
  );
}

// ── graceful shutdown ─────────────────────────────────────────────────

const shutdown = async () => {
  console.log('[choruz-bridge] Shutting down...');
  await webhookServer.stop();
  if (slackAdapter) await slackAdapter.stop();
  if (telegramAdapter) await telegramAdapter.stop();
  await mappings.close();
  process.exit(0);
};

process.on('SIGINT', () => void shutdown());
process.on('SIGTERM', () => void shutdown());

console.log('[choruz-bridge] Started');
