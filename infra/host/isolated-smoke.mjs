import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { mkdtemp, readFile, writeFile, rm, realpath } from "node:fs/promises";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const script = await realpath(process.argv[2]).catch(() => resolve(process.argv[2]));
const allowed = ["api_smoke.sh", "web_e2e.sh", "perf_ws_smoke.sh", "backup_smoke.sh", "recovery_smoke.sh"];
if (!allowed.some(name => script === join(root, "infra/host", name))) {
  throw new Error("Only the owned smoke entry points may use this launcher");
}
const owned = await mkdtemp(join(await realpath(homedir()), "choruz-smoke-"));
const configFile = join(owned, "host.env");
const config = Object.fromEntries((await readFile(join(root, "infra/host/env.example"), "utf8"))
  .split("\n").filter(line => /^[A-Z_]+=/.test(line)).map(line => {
    const index = line.indexOf("=");
    return [line.slice(0, index), line.slice(index + 1)];
  }));
const sockets = [];
let child;
let env;
const run = (binary, args, options) => new Promise((resolveExit, reject) => {
  const process = spawn(binary, args, options);
  process.once("error", reject);
  process.once("exit", code => resolveExit(code ?? 1));
});
try {
  for (const key of ["CHORUZ_PG_PORT", "CHORUZ_API_PORT", "CHORUZ_WEB_PORT", "CHORUZ_PIPELINE_METRICS_PORT"]) {
    const socket = createServer();
    sockets.push(socket);
    await new Promise((ready, reject) => {
      socket.once("error", reject);
      socket.listen(0, "127.0.0.1", ready);
    });
    config[key] = String(socket.address().port);
  }
  const base = relative(root, owned);
  Object.assign(config, {
    CHORUZ_RUNTIME_DIR: owned,
    CHORUZ_DATA_DIR: `${base}/data`,
    CHORUZ_LOG_DIR: `.choruz-runtime/logs/${base.split("/").at(-1)}`,
    CHORUZ_ATTACHMENT_DIR: `${base}/attachments`,
    CHORUZ_BACKUP_DIR: `${base}/backups`,
  });
  await writeFile(configFile, Object.entries(config).map(([key, value]) => `${key}=${value}`).join("\n") + "\n", { mode: 0o600 });
  env = { ...process.env, ...config, CHORUZ_HOST_ENV_FILE: configFile, CHORUZ_SMOKE_ENTRY: script,
    CHORUZ_CODEX_RUNTIME_DIR: join(owned, "codex-runtime"),
    CHORUZ_AGENT_TOKENS_FILE: join(owned, "agent_tokens.json"),
    CHORUZ_BACKUP_SMOKE_API_PORT: config.CHORUZ_API_PORT,
  };
  delete env.CHORUZ_DATABASE_URL;
  delete env.DATABASE_URL;
  delete env.CHORUZ_GIT_REPO_PATH;
  await Promise.all(sockets.map(socket => new Promise(done => socket.close(done))));
  child = spawn("bash", [script, ...process.argv.slice(3)], { cwd: root, env, stdio: "inherit" });
  const stop = () => child.kill("SIGTERM");
  process.once("SIGINT", stop);
  process.once("SIGTERM", stop);
  try {
    process.exitCode = await new Promise((done, reject) => {
      child.once("error", reject);
      child.once("exit", code => done(code ?? 1));
    });
  } finally {
    process.removeListener("SIGINT", stop);
    process.removeListener("SIGTERM", stop);
  }
} finally {
  for (const socket of sockets) if (socket.listening) socket.close();
  if (env) {
    const pidFile = join(owned, "data/postgres/postmaster.pid");
    const hasPostgres = () => readFile(pidFile, "utf8").then(() => true, error => {
      if (error.code === "ENOENT") return false;
      throw error;
    });
    if (await hasPostgres()) {
      await run(config.CHORUZ_PG_CTL_BIN, ["-D", join(owned, "data/postgres"), "stop", "-m", "fast", "-w"], { cwd: root, env, stdio: "ignore" });
      if (await hasPostgres()) throw new Error(`Smoke PostgreSQL did not stop; preserved ${owned}`);
    }
  }
  await rm(owned, { recursive: true, force: true });
}
