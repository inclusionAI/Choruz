const fs = require('node:fs/promises');
const path = require('node:path');
const net = require('node:net');
const { spawn } = require('node:child_process');
const { randomBytes } = require('node:crypto');
const { setTimeout: delay } = require('node:timers/promises');

// These ports are also compiled into the packaged Next.js HTTP/socket routes.
const ports = { web: 47371, api: 47372, pipeline: 47373, postgres: 47374 };
const origin = `http://127.0.0.1:${ports.web}`;

async function assertPortFree(port) {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once('error', () => reject(new Error(`Port ${port} is occupied. Close the other Choruz desktop instance and retry.`)));
    server.listen(port, '127.0.0.1', resolve);
  });
  await new Promise((resolve) => server.close(resolve));
}

async function credentials(directory) {
  const file = path.join(directory, 'credentials.json');
  try {
    await fs.writeFile(file, JSON.stringify({ session: randomBytes(32).toString('hex'), password: randomBytes(32).toString('hex') }), { flag: 'wx', mode: 0o600 });
  } catch (error) {
    if (error.code !== 'EEXIST') throw error;
  }
  const result = JSON.parse(await fs.readFile(file, 'utf8'));
  if (!/^[a-f0-9]{64}$/.test(result.session) || !/^[a-f0-9]{64}$/.test(result.password)) throw new Error('Desktop credentials are unreadable. Restore the credentials file from your backup; it has not been replaced.');
  return result;
}

class Runtime {
  constructor({ bundle, data, executable, onFailure }) {
    Object.assign(this, { bundle, data, executable, onFailure });
    this.children = [];
    this.stopping = false;
  }

  async start() {
    await fs.mkdir(this.data, { recursive: true, mode: 0o700 });
    for (const port of Object.values(ports)) await assertPortFree(port);
    const secret = await credentials(this.data);
    if (this.stopping) throw new Error('Startup cancelled.');
    const env = {
      ...process.env,
      CHORUZ_DATA_DIR: this.data,
      CHORUZ_HOST_WORKING_DIR: this.data,
      CHORUZ_RUNTIME_DIR: path.join(this.data, '.choruz-runtime'),
      CHORUZ_SESSION_SECRET: secret.session,
      CHORUZ_OPERATOR_PASSWORD: secret.password,
      CHORUZ_POSTGRES_PASSWORD: secret.password,
      CHORUZ_POSTGRES_PORT: String(ports.postgres),
      CHORUZ_API_PORT: String(ports.api),
      CHORUZ_WEB_PORT: String(ports.web),
      CHORUZ_PIPELINE_METRICS_PORT: String(ports.pipeline),
      CHORUZ_API_BASE_URL: `http://127.0.0.1:${ports.api}`,
      CHORUZ_ENV: 'production',
      PATH: `${process.env.PATH || ''}:/opt/homebrew/bin:/usr/local/bin:${path.join(require('node:os').homedir(), '.local/bin')}`,
    };
    // Never attach an installed desktop to an unrelated inherited database.
    delete env.CHORUZ_DATABASE_URL;
    this.spawn(path.join(this.bundle, 'bin/choruz-server'), [], env, 'host');
    await this.ready(`http://127.0.0.1:${ports.api}/readyz`, 180_000);
    await this.ready(`http://127.0.0.1:${ports.pipeline}/readyz`, 30_000);
    env.CHORUZ_DATABASE_URL = `postgres://postgres:${secret.password}@127.0.0.1:${ports.postgres}/choruz`;
    this.spawn(this.executable, [path.join(this.bundle, 'web/apps/web/server.js')], {
      ...env, ELECTRON_RUN_AS_NODE: '1', NODE_ENV: 'production', HOSTNAME: '127.0.0.1', PORT: String(ports.web),
    }, 'web');
    await this.ready(origin, 30_000);
    return origin;
  }

  spawn(command, args, env, name) {
    const log = require('node:fs').openSync(path.join(this.data, `${name}.log`), 'a', 0o600);
    const child = spawn(command, args, { cwd: this.data, env, stdio: ['ignore', log, log] });
    require('node:fs').closeSync(log);
    this.children.push(child);
    const failed = (message) => {
      this.failure = new Error(message);
      if (!this.stopping) this.onFailure(this.failure);
    };
    child.once('error', () => failed(`Could not launch ${name}. See ${name}.log in the desktop data folder.`));
    child.once('exit', (code, signal) => failed(`${name} stopped (${signal || code}). See ${name}.log in the desktop data folder.`));
    return child;
  }

  async ready(url, timeout) {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      if (this.stopping) throw new Error('Startup cancelled.');
      if (this.failure) throw this.failure;
      try {
        const response = await fetch(url, { redirect: 'manual', signal: AbortSignal.timeout(1000) });
        if (response.ok || response.status === 307) return;
      } catch { /* The child may not have bound its listener yet. */ }
      await delay(150);
    }
    throw new Error('Choruz startup timed out. Open the desktop data folder to inspect host.log and web.log.');
  }

  stop() {
    this.stopping = true;
    this.shutdown ??= this.stopChildren();
    return this.shutdown;
  }

  async stopChildren() {
    for (const child of [...this.children].reverse()) {
      if (!child.pid || child.exitCode !== null || child.signalCode !== null) continue;
      await new Promise((resolve) => {
        const timer = setTimeout(() => child.kill('SIGKILL'), 180_000);
        child.once('exit', () => { clearTimeout(timer); resolve(); });
        child.kill('SIGTERM');
      });
    }
    this.children = [];
  }
}

module.exports = { Runtime, ports, origin, credentials, assertPortFree };
