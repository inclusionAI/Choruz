import Fastify from 'fastify';
import { createHmac, timingSafeEqual } from 'node:crypto';
import type { MappingStore } from './mapping-store.js';
import type { SlackAdapter } from './adapters/slack.js';
import type { TelegramAdapter } from './adapters/telegram.js';
import { choruzMentionToPlatform } from './mention.js';

export interface WebhookServerConfig {
  port: number;
  secret: string;
  mappings: MappingStore;
  slack?: SlackAdapter;
  telegram?: TelegramAdapter;
}

const WEBHOOK_REPLAY_WINDOW_SECONDS = 300;
const MAX_RECENT_WEBHOOK_EVENT_IDS = 10_000;

/** Bounded, in-memory replay guard for signed webhook deliveries. */
export class WebhookReplayCache {
  private readonly eventIds = new Map<string, {
    expiresAt: number;
    running: boolean;
    completed: boolean;
    delivered: Set<string>;
  }>();

  get(eventId: string, nowSeconds = Date.now() / 1000) {
    for (const [id, entry] of this.eventIds) {
      if (!entry.running && entry.expiresAt <= nowSeconds) this.eventIds.delete(id);
    }
    const existing = this.eventIds.get(eventId);
    if (existing) return existing;
    if (this.eventIds.size >= MAX_RECENT_WEBHOOK_EVENT_IDS) {
      const evictable = [...this.eventIds].find(([, entry]) => !entry.running);
      if (!evictable) return null;
      this.eventIds.delete(evictable[0]);
    }
    const entry = { expiresAt: nowSeconds + WEBHOOK_REPLAY_WINDOW_SECONDS, running: false, completed: false, delivered: new Set<string>() };
    this.eventIds.set(eventId, entry);
    return entry;
  }
}

/** Incoming Choruz event envelope shape. */
interface ChoruzWebhookEnvelope {
  event_id: string;
  event_type: string;
  payload: {
    conversation_id: string;
    sender?: { name?: string };
    sender_id?: string;
    content: string;
    metadata?: Record<string, unknown>;
  };
}

/**
 * HTTP server that receives Choruz webhook pushes and forwards messages
 * to the corresponding Slack / Telegram channels.
 */
export class WebhookServer {
  private server = Fastify({ logger: false });
  private config: WebhookServerConfig;
  private replayCache = new WebhookReplayCache();

  constructor(config: WebhookServerConfig) {
    this.config = config;
    this.server.addContentTypeParser(
      'application/json',
      { parseAs: 'string' },
      (_request, body, done) => done(null, body),
    );
    this.registerRoutes();
  }

  // ── lifecycle ───────────────────────────────────────────────────────

  async start(): Promise<void> {
    await this.server.listen({ port: this.config.port, host: '0.0.0.0' });
    console.log(`[webhook] Listening on port ${this.config.port}`);
  }

  async stop(): Promise<void> {
    await this.server.close();
    console.log('[webhook] Server stopped');
  }

  /** Inject a request without binding a port, for integration tests. */
  async inject(options: string | {
    method?: string;
    url?: string;
    headers?: Record<string, string>;
    payload?: string;
  }) {
    return this.server.inject(options as never);
  }

  // ── routes ──────────────────────────────────────────────────────────

  private registerRoutes(): void {
    // Health check
    this.server.get('/health', async (_req, reply) => {
      return reply.code(200).send({ status: 'ok' });
    });

    // Choruz webhook endpoint
    this.server.post('/webhook/choruz', async (req, reply) => {
      const rawBody = req.body;
      const signature = req.headers['x-choruz-signature'];
      const timestamp = req.headers['x-choruz-timestamp'];
      const eventId = req.headers['x-choruz-event-id'];
      if (
        typeof rawBody !== 'string'
        || typeof signature !== 'string'
        || typeof timestamp !== 'string'
        || typeof eventId !== 'string'
        || !isCurrentWebhookTimestamp(timestamp)
        || !isValidChoruzWebhookSignature(rawBody, timestamp, signature, this.config.secret)
      ) {
        return reply.code(401).send({ error: 'invalid webhook signature' });
      }

      let payload: ChoruzWebhookEnvelope;
      try {
        payload = JSON.parse(rawBody) as ChoruzWebhookEnvelope;
      } catch {
        return reply.code(400).send({ error: 'invalid payload' });
      }

      if (
        !payload
        || !payload.event_id
        || payload.event_id !== eventId
        || !payload.event_type
        || !payload.payload
      ) {
        return reply.code(400).send({ error: 'invalid payload' });
      }

      // Only handle message.created events
      if (payload.event_type !== 'message.created') {
        return reply.code(200).send({ status: 'ignored', reason: 'event_type' });
      }

      // Loop prevention: skip messages originating from the bridge itself
      if (payload.payload.metadata?.['bridge_platform']) {
        return reply
          .code(200)
          .send({ status: 'ignored', reason: 'bridge_originated' });
      }

      const { conversation_id, content } = payload.payload;
      const senderName = payload.payload.sender?.name
        ?? payload.payload.sender_id
        ?? 'Unknown sender';
      if (!conversation_id || !content) {
        return reply.code(400).send({ error: 'missing conversation_id or content' });
      }

      const delivery = this.replayCache.get(payload.event_id);
      if (!delivery || delivery.running) {
        return reply.code(503).send({ status: 'retry', reason: 'delivery_busy' });
      }
      if (delivery.completed) {
        return reply.code(200).send({ status: 'ignored', reason: 'duplicate_event' });
      }
      delivery.running = true;
      try {

        console.log(
          `[webhook] message.created in ${conversation_id} from ${senderName}`,
        );

        // Look up all platform channels mapped to this Choruz conversation
        const channels =
          await this.config.mappings.findByChoruzConversation(conversation_id);

        if (channels.length === 0) {
          return reply
            .code(200)
            .send({ status: 'ignored', reason: 'no_mapping' });
        }

        const platformContent = choruzMentionToPlatform(content);

        // Fan out to each mapped platform channel
        const results: Array<{ platform: string; channel: string; ok: boolean }> =
          [];

        for (const ch of channels) {
          const channelKey = JSON.stringify([ch.platform, ch.platform_channel_id]);
          if (delivery.delivered.has(channelKey)) {
            results.push({ platform: ch.platform, channel: ch.platform_channel_id, ok: true });
            continue;
          }
          try {
            if (ch.platform === 'slack' && this.config.slack) {
              await this.config.slack.pushToSlack(
                ch.platform_channel_id,
                platformContent,
                senderName,
              );
              results.push({
                platform: 'slack',
                channel: ch.platform_channel_id,
                ok: true,
              });
            } else if (ch.platform === 'telegram' && this.config.telegram) {
              await this.config.telegram.pushToTelegram(
                ch.platform_channel_id,
                platformContent,
                senderName,
              );
              results.push({
                platform: 'telegram',
                channel: ch.platform_channel_id,
                ok: true,
              });
            } else {
              console.warn(
                `[webhook] No adapter for platform "${ch.platform}", channel ${ch.platform_channel_id}`,
              );
              results.push({
                platform: ch.platform,
                channel: ch.platform_channel_id,
                ok: false,
              });
            }
          } catch (err) {
            console.error(
              `[webhook] Failed to push to ${ch.platform}/${ch.platform_channel_id}:`,
              err,
            );
            results.push({
              platform: ch.platform,
              channel: ch.platform_channel_id,
              ok: false,
            });
          }
          if (results.at(-1)?.ok) delivery.delivered.add(channelKey);
        }

        delivery.completed = results.every((result) => result.ok);
        return reply.code(delivery.completed ? 200 : 503).send({ status: delivery.completed ? 'delivered' : 'partial_failure', results });
      } finally {
        delivery.running = false;
      }
    });
  }
}

export function isValidChoruzWebhookSignature(
  rawBody: string,
  timestamp: string,
  signature: string,
  secret: string,
): boolean {
  const expected = `sha256=${createHmac('sha256', secret)
    .update(timestamp)
    .update('.')
    .update(rawBody)
    .digest('hex')}`;
  const expectedBuffer = Buffer.from(expected);
  const signatureBuffer = Buffer.from(signature);
  return expectedBuffer.length === signatureBuffer.length
    && timingSafeEqual(expectedBuffer, signatureBuffer);
}

function isCurrentWebhookTimestamp(timestamp: string): boolean {
  const parsed = Number(timestamp);
  return Number.isSafeInteger(parsed)
    && Math.abs(Date.now() / 1000 - parsed) <= WEBHOOK_REPLAY_WINDOW_SECONDS;
}
