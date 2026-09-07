import assert from "node:assert/strict";
import { spawn, execFile } from "node:child_process";
import { open, mkdir, readFile, writeFile } from "node:fs/promises";
import { createServer, connect } from "node:net";
import { createRequire } from "node:module";
import { promisify } from "node:util";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const require = createRequire(new URL("../../apps/web/package.json", import.meta.url));
const { Client } = require("pg");
const exec = promisify(execFile);
const root = resolve(import.meta.dirname, "../..");
const owned = process.env.CHORUZ_RUNTIME_DIR;
assert.equal(process.env.CHORUZ_SMOKE_ENTRY, join(root, "infra/host/recovery_smoke.sh"));
assert.ok(owned && owned !== root);
const base = `http://127.0.0.1:${process.env.CHORUZ_API_PORT}`;
const ready = `http://127.0.0.1:${process.env.CHORUZ_PIPELINE_METRICS_PORT}/readyz`;
const database = `postgres://${process.env.CHORUZ_PG_USER}@127.0.0.1:${process.env.CHORUZ_PG_PORT}/${process.env.CHORUZ_PG_DB}`;
const db = new Client({ connectionString: database });
const children = [];
const logs = [];
const sockets = new Set();
let blocked = false;
let fixturePid;
let token;
let interrupted = false;
const interrupt = () => { interrupted = true; };
process.on("SIGTERM", interrupt);
process.on("SIGINT", interrupt);
const workspace = join(owned, "recovery-workspace");
const fixture = join(workspace, "fixture-cli.mjs");

async function until(description, check, timeout = 60_000, observeInterrupt = true) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (observeInterrupt && interrupted) throw new Error("Recovery smoke interrupted");
    const result = await check();
    if (result) return result;
    await delay(100);
  }
  throw new Error(`Timed out: ${description}`);
}
async function api(path, body) {
  const response = await fetch(`${base}${path}`, {
    method: body === undefined ? "GET" : "POST",
    headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(10_000),
  });
  assert.ok(response.ok, `${path}: ${response.status} ${await response.clone().text()}`);
  return response.json();
}
async function launch(binary, extra) {
  const log = await open(join(owned, `${binary}.log`), "w");
  logs.push(log);
  const child = spawn(join(root, "target/debug", binary), [], {
    cwd: root, env: { ...process.env, RUST_LOG: "info", CHORUZ_DATABASE_URL: database, ...extra },
    stdio: ["ignore", log.fd, log.fd],
  });
  const exited = new Promise(resolveExit => {
    child.once("exit", resolveExit);
    child.once("error", resolveExit);
  });
  children.push({ child, exited });
}
const proxy = createServer(client => {
  if (blocked) return client.destroy();
  const upstream = connect(Number(process.env.CHORUZ_PG_PORT), "127.0.0.1");
  for (const socket of [client, upstream]) {
    sockets.add(socket);
    socket.on("error", () => { client.destroy(); upstream.destroy(); });
    socket.on("close", () => { sockets.delete(socket); client.destroy(); upstream.destroy(); });
  }
  client.pipe(upstream).pipe(client);
});
async function killFixture() {
  if (!fixturePid) return;
  const command = await exec("ps", ["-p", String(fixturePid), "-o", "command="]).catch(() => null);
  if (command) {
    assert.ok(command.stdout.includes(fixture), "Refuse to stop a PID not owned by this fixture");
    process.kill(fixturePid, "SIGKILL");
    await until("owned CLI exit", async () => !(await exec("ps", ["-p", String(fixturePid), "-o", "command="]).catch(() => null)), 5000, false);
  }
  fixturePid = undefined;
}
try {
  await mkdir(workspace);
  await writeFile(fixture, `#!${process.execPath}
import { existsSync, writeFileSync, readFileSync } from 'node:fs';
const marker = ${JSON.stringify(join(workspace, "first-attempt"))};
if (!existsSync(marker)) {
  writeFileSync(marker, String(process.pid));
  setInterval(() => {}, 1000);
} else {
  console.log(JSON.stringify({type:'result', result:readFileSync(${JSON.stringify(join(workspace, "reply"))}, 'utf8')}));
}
`, { mode: 0o700 });
  await writeFile(join(workspace, "reply"), "process recovery reply");
  await db.connect();
  await new Promise((resolveListen, reject) => { proxy.once("error", reject); proxy.listen(0, "127.0.0.1", resolveListen); });
  await launch("choruz-api-gateway", {});
  await until("API startup", () => fetch(`${base}/healthz`).then(r => r.ok, () => false));
  await launch("choruz-pipeline", {
    CHORUZ_DATABASE_URL: database.replace(`:${process.env.CHORUZ_PG_PORT}/`, `:${proxy.address().port}/`),
    CHORUZ_API_BASE_URL: base, CHORUZ_CLAUDE_CLI_PATH: fixture,
    CHORUZ_PIPELINE_RETRY_CHECK_MS: "100",
  });
  await until("pipeline startup", () => fetch(ready).then(r => r.ok, () => false));
  console.log(JSON.stringify({ owned, processes: children.map(({ child }) => child.pid) }));
  const login = await api("/v1/auth/local/login", { username: process.env.CHORUZ_OPERATOR_USER, password: process.env.CHORUZ_OPERATOR_PASSWORD });
  token = login.session_token;
  const actor = login.principal.id;
  const agent = await api("/v1/agents", { actor_id: actor, name: "recovery-agent", scopes: [] });
  const conversation = await api("/v1/conversations/direct", { actor_id: actor, peer_principal_id: agent.principal.id });
  await api("/v1/runtime/bindings", { conversation_id: conversation.id, agent_principal_id: agent.principal.id, driver_type: "claude_print", workspace_path: workspace });
  async function send(content) {
    return api("/v1/messages", { actor_id: actor, conversation_id: conversation.id, content, content_type: "text", metadata: {}, idempotency_key: content });
  }
  async function command(message) {
    return (await db.query("SELECT command_id, status, attempt_count FROM agent_commands WHERE message_id=$1", [message.id])).rows[0];
  }
  async function delivered(message, content) {
    const state = await command(message);
    const messages = await api(`/v1/conversations/${conversation.id}/messages?principal_id=${actor}`);
    const matching = messages.filter(message => message.sender_id === agent.principal.id && message.content === content);
    return state?.status === "committed" && matching.length === 1 ? state : false;
  }
  const first = await send("process loss request");
  fixturePid = Number(await until("owned CLI ready", () => readFile(join(workspace, "first-attempt"), "utf8").catch(() => false)));
  const before = await command(first);
  assert.ok(before?.command_id);
  console.log(JSON.stringify({ scenario: "process-loss", message: first.id, before }));
  await killFixture();
  const recovered = await until("retried command and persisted reply", () => delivered(first, "process recovery reply"));
  assert.equal(recovered.command_id, before.command_id);
  assert.ok(recovered.attempt_count > before.attempt_count);
  console.log(JSON.stringify({ scenario: "process-loss", recovered }));

  blocked = true;
  for (const socket of sockets) socket.destroy();
  await until("database partition observed", () => fetch(ready, { signal: AbortSignal.timeout(5000) }).then(r => r.status === 503, () => false));
  await writeFile(join(workspace, "reply"), "network recovery reply");
  const second = await send("network loss request");
  console.log(JSON.stringify({ scenario: "database-partition", message: second.id, fault: "proxy-disconnected" }));
  blocked = false;
  const network = await until("partition recovery and persisted reply", () => delivered(second, "network recovery reply"));
  console.log(JSON.stringify({ scenario: "database-partition", recovered: network }));
  console.log("owned recovery smoke passed");
} catch (error) {
  for (const binary of ["choruz-api-gateway", "choruz-pipeline"]) {
    console.error((await readFile(join(owned, `${binary}.log`), "utf8").catch(() => "")).split("\n").slice(-50).join("\n"));
  }
  throw error;
} finally {
  try {
    await killFixture();
  } finally {
    for (const { child } of children) if (child.exitCode === null) child.kill("SIGTERM");
    for (const { child, exited } of children) {
      const timer = setTimeout(() => child.kill("SIGKILL"), 5000);
      await exited;
      clearTimeout(timer);
    }
    for (const socket of sockets) socket.destroy();
    if (proxy.listening) await new Promise(done => proxy.close(done));
    await db.end();
    for (const log of logs) await log.close();
    process.removeListener("SIGTERM", interrupt);
    process.removeListener("SIGINT", interrupt);
  }
}
