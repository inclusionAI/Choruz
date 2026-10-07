const fs = require('node:fs/promises');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { ports } = require('../runtime.cjs');

const root = path.resolve(__dirname, '../../..');
const bundle = path.resolve(__dirname, '../bundle');

function run(command, args, env = {}) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit', env: { ...process.env, ...env } });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed (${result.status})`);
}

async function prepare() {
  run('cargo', ['build', '--release', '-p', 'choruz-server', '-p', 'choruz-api-gateway', '-p', 'choruz-pipeline', '-p', 'choruz-connector']);
  run('pnpm', ['web:build'], { CHORUZ_API_PORT: String(ports.api), NEXT_PUBLIC_CHORUZ_API_PORT: String(ports.api), CHORUZ_API_BASE_URL: `http://127.0.0.1:${ports.api}` });
  await fs.rm(bundle, { recursive: true, force: true });
  await fs.mkdir(path.join(bundle, 'bin'), { recursive: true });
  for (const binary of ['choruz-server', 'choruz-api-gateway', 'choruz-pipeline', 'choruz-connector']) {
    await fs.copyFile(path.join(root, 'target/release', binary), path.join(bundle, 'bin', binary));
    await fs.chmod(path.join(bundle, 'bin', binary), 0o755);
  }
  await fs.cp(path.join(root, 'migrations'), path.join(bundle, 'bin/migrations'), { recursive: true });
  const standalone = path.join(root, 'apps/web/.next/standalone');
  await fs.cp(standalone, path.join(bundle, 'web'), {
    recursive: true, dereference: false, verbatimSymlinks: true,
    filter(source) {
      const relative = path.relative(standalone, source).split(path.sep).join('/');
      return !relative || ['apps', 'apps/web'].includes(relative)
        || relative === 'apps/web/server.js' || relative === 'apps/web/package.json'
        || ['node_modules', 'apps/web/node_modules', 'apps/web/.next'].some((directory) => relative === directory || relative.startsWith(`${directory}/`));
    },
  });
  await fs.cp(path.join(root, 'apps/web/.next/static'), path.join(bundle, 'web/apps/web/.next/static'), { recursive: true });
  await fs.cp(path.join(root, 'apps/web/public'), path.join(bundle, 'web/apps/web/public'), { recursive: true });
  await fs.cp(path.join(root, 'crates/choruz-host-runtime/assets/agent-templates'), path.join(bundle, 'web/crates/choruz-host-runtime/assets/agent-templates'), { recursive: true });
  await fs.mkdir(path.join(bundle, 'web/scripts'), { recursive: true });
  await fs.copyFile(path.join(root, 'crates/choruz-host-runtime/assets/choruz-send.sh'), path.join(bundle, 'web/scripts/choruz-send.sh'));
  await fs.copyFile(path.join(root, 'LICENSE'), path.join(bundle, 'LICENSE'));
  await require('sharp')(path.join(root, 'assets/brand/signal-chorus-mark.svg'), { density: 300 }).resize(1024, 1024).png().toFile(path.join(bundle, 'icon.png'));
  console.log(`Desktop runtime prepared at ${bundle}`);
}

prepare().catch((error) => { console.error(error.message); process.exitCode = 1; });
