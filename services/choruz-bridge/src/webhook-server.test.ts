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
  it('delivers each signed event ID only once', async () => {
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
    const deliveries: Array<{ content: string; senderName: string }> = [];
    const server = new WebhookServer({
      port: 0,
      secret,
      mappings: {
        findByChoruzConversation: async () => [{
          platform: 'slack',
          platform_channel_id: 'channel-1',
          platform_channel_name: null,
        }],
      } as never,
      slack: {
        pushToSlack: async (_channel, content, senderName) => {
          deliveries.push({ content, senderName });
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

    expect((await server.inject(request)).statusCode).toBe(200);
    expect((await server.inject({
      ...request,
      headers: { ...request.headers, 'x-choruz-event-id': 'tampered-event-id' },
    })).statusCode).toBe(400);
    expect((await server.inject(request)).json()).toMatchObject({
      status: 'ignored', reason: 'duplicate_event',
    });
    expect(deliveries).toEqual([{ content: 'hello', senderName: 'Ada' }]);
    await server.stop();
  });
});
