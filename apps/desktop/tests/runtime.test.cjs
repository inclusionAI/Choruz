const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');
const { Runtime, credentials, assertPortFree } = require('../runtime.cjs');

test('desktop identity persists and never silently replaces an invalid identity', async (t) => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'choruz-desktop-'));
  t.after(() => fs.rm(directory, { recursive: true, force: true }));
  const first = await credentials(directory);
  assert.deepEqual(await credentials(directory), first);
  assert.equal((await fs.stat(path.join(directory, 'credentials.json'))).mode & 0o777, 0o600);
  await fs.writeFile(path.join(directory, 'credentials.json'), '{}');
  await assert.rejects(credentials(directory), /not been replaced/);
  assert.equal(await fs.readFile(path.join(directory, 'credentials.json'), 'utf8'), '{}');
});

test('occupied port is rejected without stopping its owner', async (t) => {
  const server = net.createServer();
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise((resolve) => server.close(resolve)));
  await assert.rejects(assertPortFree(server.address().port), /occupied/);
  assert.equal(server.listening, true);
});

test('runtime stop reaps its child without reporting an unexpected failure', async (t) => {
  const data = await fs.mkdtemp(path.join(os.tmpdir(), 'choruz-desktop-'));
  t.after(() => fs.rm(data, { recursive: true, force: true }));
  const failures = [];
  const runtime = new Runtime({ data, onFailure: (error) => failures.push(error) });
  t.after(() => runtime.stop());
  const child = runtime.spawn(process.execPath, ['-e', 'setInterval(()=>{},1000)'], process.env, 'fixture');
  await new Promise((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject); });
  await runtime.stop();
  assert.notEqual(child.signalCode, null);
  assert.equal(runtime.children.length, 0);
  assert.equal(failures.length, 0);
});

test('spawn failure can be shut down without waiting for an exit that never occurs', async (t) => {
  const data = await fs.mkdtemp(path.join(os.tmpdir(), 'choruz-desktop-'));
  t.after(() => fs.rm(data, { recursive: true, force: true }));
  let notify;
  const failed = new Promise((resolve) => { notify = resolve; });
  const runtime = new Runtime({ data, onFailure: notify });
  runtime.spawn(path.join(data, 'missing'), [], process.env, 'fixture');
  await failed;
  await runtime.stop();
  assert.equal(runtime.children.length, 0);
});
