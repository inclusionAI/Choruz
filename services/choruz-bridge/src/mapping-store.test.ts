import { afterEach, expect, it, vi } from 'vitest';
import { MappingStore } from './mapping-store.js';
import { SlackAdapter } from './adapters/slack.js';
import { TelegramAdapter } from './adapters/telegram.js';

const database = vi.hoisted(() => ({ query: vi.fn(), release: vi.fn() }));
const commands = vi.hoisted(() => new Map<string, (context: never) => Promise<void>>());
vi.mock('@slack/bolt', () => ({ App: class {
  message() {}
  command(name: string, callback: (context: never) => Promise<void>) { commands.set(name, callback); }
} }));
vi.mock('grammy', () => ({ Bot: class {
  on() {}
  command(name: string, callback: (context: never) => Promise<void>) { commands.set(name, callback); }
} }));
vi.mock('pg', () => ({ default: { Pool: class {
  connect = async () => database;
  end = async () => {};
} } }));
afterEach(() => vi.clearAllMocks());

it.each(['slack', 'telegram'])('%s group commands announce the existing authoritative link', async (platform) => {
  database.query.mockImplementation(async (sql: string) => ({ rows: sql.startsWith('SELECT choruz_conversation') ? [{ choruz_conversation_id: 'existing-group' }] : [] }));
  const createGroup = vi.fn(async () => ({ id: 'unlinked-replacement' }));
  const params = { botToken: 'fixture', appToken: 'fixture', mappings: new MappingStore('fixture'), choruz: { createGroup } as never, bridgePrincipalId: 'operator' };
  const reply = vi.fn();
  if (platform === 'slack') {
    new SlackAdapter(params);
    await commands.get('/new-group')!({ command: { text: 'new-name', channel_id: 'channel', channel_name: 'name' }, ack: vi.fn(), respond: reply } as never);
    expect(reply.mock.calls[0][0].text).toContain('existing-group');
  } else {
    new TelegramAdapter(params);
    await commands.get('new_group')!({ match: 'new-name', chat: { id: 'channel', title: 'name' }, reply } as never);
    expect(reply.mock.calls[0][0]).toContain('existing-group');
  }
  expect(createGroup).not.toHaveBeenCalled();
  expect(JSON.stringify(reply.mock.calls)).not.toContain('unlinked-replacement');
  expect(database.release).toHaveBeenCalledOnce();
});

it('does not report a link when its durable insert fails and releases the transaction', async () => {
  database.query.mockImplementation(async (sql: string) => {
    if (sql.startsWith('INSERT')) throw new Error('storage unavailable');
    return { rows: [] };
  });
  const store = new MappingStore('fixture');
  await expect(store.getOrCreate('telegram', 'chat', 'name', async () => 'new-group')).rejects.toThrow('storage unavailable');
  expect(database.query).toHaveBeenLastCalledWith('ROLLBACK');
  expect(database.query.mock.calls.some(([sql]) => sql === 'COMMIT')).toBe(false);
  expect(database.release).toHaveBeenCalledOnce();
});
