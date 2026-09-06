import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const root = fileURLToPath(new URL("../../", import.meta.url));
const require = createRequire(new URL("../../services/remote-control-gateway/package.json", import.meta.url));
const { unstable_dev, getPlatformProxy } = require("wrangler");
const secret = randomBytes(32).toString("hex");
const stateDir = await mkdtemp(join(tmpdir(), "choruz-online-e2e-"));
let worker;
try {
  const platform = await getPlatformProxy({
    configPath: `${root}services/remote-control-gateway/wrangler.toml`,
    persist: { path: join(stateDir, "v3") }, remoteBindings: false,
  });
  try {
    const migrations = `${root}services/remote-control-gateway/migrations`;
    for (const file of (await readdir(migrations)).filter(name => name.endsWith(".sql")).sort()) {
      await platform.env.ONLINE_DATABASE.exec(await readFile(join(migrations, file), "utf8"));
    }
  } finally { await platform.dispose(); }
  worker = await unstable_dev(`${root}services/remote-control-gateway/src/index.ts`, {
  config: `${root}services/remote-control-gateway/wrangler.toml`,
  ip: "127.0.0.1", port: 0, inspectorPort: 0, persist: true, persistTo: stateDir,
  vars: { GATEWAY_AUTH_SECRET: secret, ONLINE_AUTH_SECRET: secret },
  logLevel: "error",
  experimental: { disableExperimentalWarning: true, disableDevRegistry: true, watch: false },
});

  const gateway = `http://127.0.0.1:${worker.port}`;
  const ready = await fetch(`${gateway}/healthz`);
  if (!ready.ok) throw new Error(`Remote test Worker readiness returned ${ready.status}`);
  const child = spawn("bash", [`${root}infra/host/web_e2e.sh`, ...process.argv.slice(2)], {
    stdio: "inherit",
    env: {
      ...process.env,
      CHORUZ_REMOTE_E2E_RUNNING: "1",
      CHORUZ_REMOTE_CONTROL_GATEWAY_URL: gateway,
      CHORUZ_REMOTE_CONTROL_GATEWAY_SECRET: secret,
      CHORUZ_ONLINE_URL: gateway,
    },
  });
  const stop = () => child.kill("SIGTERM");
  process.once("SIGINT", stop);
  process.once("SIGTERM", stop);
  try {
    process.exitCode = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("exit", (code) => resolve(code ?? 1));
    });
  } finally {
    process.removeListener("SIGINT", stop);
    process.removeListener("SIGTERM", stop);
  }
} finally {
  await worker?.stop();
  await rm(stateDir, { recursive: true, force: true });
}
