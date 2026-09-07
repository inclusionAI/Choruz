import { createHmac } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import { WebhookServer, isValidChoruzWebhookSignature } from './webhook-server.js';

describe('isValidChoruzWebhookSignature', () => {
  it('accepts only the HMAC for the exact raw payload', () => {
    const body = '{"event_type":"message.created"}';
    const timestamp = '123';
    const secret = 'test-secret';
    const signature = `sha256=${createHmac('sha256', secret)
      .update(timestamp)
      .update('.')
      .update(body)
      .digest('hex')}`;

    expect(isValidChoruzWebhookSignature(body, timestamp, signature, secret)).toBe(true);
    expect(isValidChoruzWebhookSignature(`${body} `, timestamp, signature, secret)).toBe(false);
    expect(isValidChoruzWebhookSignature(body, '124', signature, secret)).toBe(false);
  });
});

describe('WebhookServer', () => {
  it('retries failed channels without duplicating completed or in-flight delivery', async () => {
    const secret = 'test-secret';
    const timestamp = String(Math.floor(Date.now() / 1000));
    const body = JSON.stringify({
      delivery_seq: 1,
      event_id: 'event-1',
      principal_id: 'principal-1',
      event_type: 'message.created',
      created_at: '2026-07-20T09:00:00Z',
      payload: {
        conversation_id: 'conversation-1',
        sender: { id: 'principal-2', name: 'Ada', type: 'human' },
        sender_id: 'principal-2',
        content: 'hello',
        metadata: {},
      },
    });
    const signature = `sha256=${createHmac('sha256', secret)
      .update(timestamp)
      .update('.')
      .update(body)
      .digest('hex')}`;
    const deliveries: Array<{ channel: string; content: string; senderName: string }> = [];
    let failSecond = true;
    let entered!: () => void;
    const started = new Promise<void>((resolve) => { entered = resolve; });
    let finish!: () => void;
    const held = new Promise<void>((resolve) => { finish = resolve; });
    const server = new WebhookServer({
      port: 0,
      secret,
      mappings: {
        findByChoruzConversation: async () => [{
          platform: 'slack',
          platform_channel_id: 'channel-1',
          platform_channel_name: null,
        }, { platform: 'slack', platform_channel_id: 'channel-2', platform_channel_name: null }],
      } as never,
      slack: {
        pushToSlack: async (channel: string, content: string, senderName: string) => {
          if (channel === 'channel-1') { entered(); await held; }
          if (channel === 'channel-2' && failSecond) { failSecond = false; throw new Error('temporary platform outage'); }
          deliveries.push({ channel, content, senderName });
        },
      } as never,
    });
    const request = {
      method: 'POST' as const,
      url: '/webhook/choruz',
      payload: body,
      headers: {
        'content-type': 'application/json',
        'x-choruz-event-id': 'event-1',
        'x-choruz-timestamp': timestamp,
        'x-choruz-signature': signature,
      },
    };

    const first = server.inject(request);
    try {
      await started;
      expect((await server.inject(request)).statusCode).toBe(503);
      finish();
      const partial = await first;
      expect(partial.statusCode).toBe(503);
      expect(partial.json()).toMatchObject({ status: 'partial_failure' });
      expect((await server.inject(request)).statusCode).toBe(200);
      expect((await server.inject({
        ...request,
        headers: { ...request.headers, 'x-choruz-event-id': 'tampered-event-id' },
      })).statusCode).toBe(400);
      expect((await server.inject(request)).json()).toMatchObject({
        status: 'ignored', reason: 'duplicate_event',
      });
      expect(deliveries).toEqual([
        { channel: 'channel-1', content: 'hello', senderName: 'Ada' },
        { channel: 'channel-2', content: 'hello', senderName: 'Ada' },
      ]);
    } finally {
      finish();
      await first;
      await server.stop();
    }
  });
});
