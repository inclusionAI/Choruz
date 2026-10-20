import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import test from 'node:test';
import pg from 'pg';
import { MappingStore } from '../src/mapping-store.js';

test('concurrent bridge processes create one canonical channel link', async () => {
  const connection = process.env.CHORUZ_TEST_DATABASE_URL;
  assert.ok(connection || process.env.PGDATABASE, 'CHORUZ_TEST_DATABASE_URL (URI) or PGDATABASE is required');
  const schema = `bridge_${randomUUID().replaceAll('-', '')}`;
  const config = connection ? { connectionString: connection } : {};
  const admin = new pg.Pool(config);
  const scoped = { ...config, options: `-c search_path=${schema}`, application_name: schema };
  const first = new MappingStore(scoped);
  const second = new MappingStore(scoped);
  let release!: () => void;
  const held = new Promise<void>((resolve) => { release = resolve; });
  let entered!: () => void;
  const started = new Promise<void>((resolve) => { entered = resolve; });
  const pending: Array<Promise<string>> = [];
  try {
    await admin.query(`CREATE SCHEMA ${schema}`);
    await admin.query(`CREATE TABLE ${schema}.bridge_channel_mappings (
      platform text NOT NULL, platform_channel_id text NOT NULL,
      choruz_conversation_id text NOT NULL, platform_channel_name text,
      PRIMARY KEY(platform, platform_channel_id))`);
    let factories = 0;
    pending.push(first.getOrCreate('slack', 'owned-channel', 'first', async () => {
      factories++; entered(); await held; return 'canonical';
    }));
    await started;
    pending.push(second.getOrCreate('slack', 'owned-channel', 'second', async () => {
      factories++; return 'wrong-replacement';
    }));
    // Observe the second database session blocked on the first transaction,
    // rather than guessing overlap from elapsed time or repeated execution.
    const deadline = Date.now() + 5000;
    let waiting = false;
    while (!waiting && Date.now() < deadline) {
      const result = await admin.query("SELECT 1 FROM pg_stat_activity WHERE application_name=$1 AND wait_event_type='Lock'", [schema]);
      waiting = result.rowCount === 1;
    }
    assert.ok(waiting, 'second bridge must wait for the channel owner');
    release();
    assert.deepEqual(await Promise.all(pending), ['canonical', 'canonical']);
    assert.equal(factories, 1);
    assert.equal(await second.getOrCreate('slack', 'owned-channel', 'third', async () => { throw new Error('existing link must not create a group'); }), 'canonical');
    const rows = await admin.query(`SELECT choruz_conversation_id FROM ${schema}.bridge_channel_mappings`);
    assert.deepEqual(rows.rows, [{ choruz_conversation_id: 'canonical' }]);
  } finally {
    release();
    await Promise.allSettled(pending);
    await Promise.all([first.close(), second.close()]);
    await admin.query(`DROP SCHEMA IF EXISTS ${schema} CASCADE`);
    await admin.end();
  }
});
