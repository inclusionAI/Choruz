const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { Runtime, ports, credentials, assertPortFree } = require('../runtime.cjs');

async function smoke() {
  const bundle = path.resolve(process.argv[2] || path.join(__dirname, '../bundle'));
  const data = await fs.mkdtemp(path.join(os.tmpdir(), 'choruz-desktop-smoke-'));
  console.log(`Isolated acceptance data: ${data}`);
  const failures = [];
  let runtime;
  try {
    for (let launch = 0; launch < 2; launch++) {
      runtime = new Runtime({ bundle, data, executable: process.execPath, onFailure: (error) => failures.push(error) });
      const origin = await runtime.start();
      const secret = await credentials(data);
      const response = await fetch(`http://127.0.0.1:${ports.api}/v1/auth/local/login`, {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ username: 'operator', password: secret.password }),
      });
      assert.equal(response.status, 200);
      const { session_token } = await response.json();
      const dashboard = await fetch(`${origin}/dashboard`, { headers: { cookie: `choruz_session=${session_token}` } });
      assert.equal(dashboard.status, 200);
      assert.match(await dashboard.text(), /What would you like to work on/);
      const proxied = await fetch(`${origin}/api/v1/bootstrap`, { headers: { cookie: `choruz_session=${session_token}` } });
      assert.equal(proxied.status, 200);
      await runtime.stop();
      for (const port of Object.values(ports)) await assertPortFree(port);
      assert.deepEqual(failures, []);
      console.log(`Launch ${launch + 1}: authenticated workbench, proxy and owned shutdown passed.`);
    }
  } finally {
    await runtime?.stop();
  }
  await fs.rm(data, { recursive: true, force: true });
}

smoke().catch((error) => { console.error(error.message); process.exitCode = 1; });
