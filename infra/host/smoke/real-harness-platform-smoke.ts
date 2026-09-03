#!/usr/bin/env node
import { access, mkdtemp, mkdir, realpath, rm, writeFile } from "node:fs/promises";
import { constants as fsConstants } from "node:fs";
import { homedir } from "node:os";
import { basename, isAbsolute, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";

export type Harness = "claude" | "codex" | "pi" | "grok" | "opencode";
export type Verdict = "PASS" | "FAIL" | "BLOCKED" | "SKIP";
export type HarnessScenario = "model-discovery" | "provision-dm" | "restart-resume" | "group-two-helper-sends" | "scan-import-context";
export type Scenario = HarnessScenario | "authentication" | "setup" | "cleanup";
type Result = { harness: Harness | "platform"; scenario: Scenario; verdict: Verdict; reason: string };
type JsonObject = Record<string, unknown>;
type NativeSeed = { nativeSessionId: string; stdout: string; stderr: string };
type Provisioned = { harness: Harness; agentId: string; conversationId: string; sessionId?: string };
type Seeded = { harness: Harness; workspace: string; marker: string; nativeSessionId: string };

const HARNESS_SCENARIOS: readonly HarnessScenario[] = ["model-discovery", "provision-dm", "restart-resume", "group-two-helper-sends", "scan-import-context"];
const SUPPORTED_HARNESSES: readonly Harness[] = ["claude", "codex", "pi", "grok", "opencode"];

export class ResultBook {
  readonly #entries = new Map<string, Result>();
  constructor(harnesses: readonly Harness[]) {
    for (const harness of harnesses) for (const scenario of HARNESS_SCENARIOS) this.#entries.set(`${harness}:${scenario}`, { harness, scenario, verdict: "FAIL", reason: "scenario-not-executed" });
    for (const scenario of ["authentication", "setup", "cleanup"] as const) this.#entries.set(`platform:${scenario}`, { harness: "platform", scenario, verdict: "SKIP", reason: "not-required" });
  }
  set(harness: Harness | "platform", scenario: Scenario, verdict: Verdict, reason: string): void {
    if (!/^[a-z0-9-]+$/.test(reason)) throw new Error(`unsafe reason code: ${reason}`);
    const key = `${harness}:${scenario}`;
    if (!this.#entries.has(key)) throw new Error(`unknown scenario: ${key}`);
    this.#entries.set(key, { harness, scenario, verdict, reason });
  }
  fillHarness(harness: Harness, verdict: Verdict, reason: string): void {
    for (const scenario of HARNESS_SCENARIOS) if (this.#entries.get(`${harness}:${scenario}`)?.reason === "scenario-not-executed") this.set(harness, scenario, verdict, reason);
  }
  entries(): Result[] { return [...this.#entries.values()]; }
  exitCode(): number {
    const entries = this.entries();
    if (entries.some((entry) => entry.verdict === "FAIL")) return 1;
    if (entries.some((entry) => entry.verdict === "BLOCKED" || entry.verdict === "SKIP")) return 3;
    return 0;
  }
}

export class HttpError extends Error {
  readonly status: number;
  readonly body: string;
  constructor(status: number, body: string, message = `HTTP ${status}`) { super(message); this.status = status; this.body = body; }
}
class StopRun extends Error {}
export function classifyHttpError(error: unknown): { verdict: Verdict; reason: string } {
  if (error instanceof HttpError && (error.status === 401 || error.status === 403)) return { verdict: "BLOCKED", reason: "operator-auth-unavailable" };
  return { verdict: "FAIL", reason: "platform-request-failed" };
}
export function isExplicitHarnessAuthError(stderr: string): boolean {
  return /(?:not logged in|authentication required|invalid api key|unauthorized|please run \/login)/i.test(stderr);
}

function scrubbedChildEnv(): NodeJS.ProcessEnv {
  const env = { ...process.env };
  for (const key of Object.keys(env)) if (key.startsWith("CHORUZ_SMOKE_") || key.startsWith("CHORUZ_REAL_HARNESS_")) delete env[key];
  return env;
}
type CommandResult = { code: number | null; stdout: string; stderr: string };
export async function runCommand(command: string, args: string[], options: { cwd: string; input?: string; timeoutMs?: number; env?: NodeJS.ProcessEnv }): Promise<CommandResult> {
  return await new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { cwd: options.cwd, env: { ...scrubbedChildEnv(), ...options.env }, stdio: ["pipe", "pipe", "pipe"], shell: false });
    let stdout = ""; let stderr = ""; let timedOut = false;
    let killTimer: NodeJS.Timeout | undefined;
    const timer = setTimeout(() => { timedOut = true; child.kill("SIGTERM"); killTimer = setTimeout(() => child.kill("SIGKILL"), 5_000); }, options.timeoutMs ?? 120_000);
    const clearTimers = () => { clearTimeout(timer); if (killTimer) clearTimeout(killTimer); };
    child.once("error", (error) => { clearTimers(); reject(error); });
    child.stdout.setEncoding("utf8").on("data", (chunk) => { stdout += chunk; });
    child.stderr.setEncoding("utf8").on("data", (chunk) => { stderr += chunk; });
    child.once("close", (code) => { clearTimers(); resolvePromise({ code: timedOut ? null : code, stdout, stderr }); });
    child.stdin.on("error", () => {});
    child.stdin.end(options.input ?? "");
  });
}

interface HarnessAdapter {
  readonly harness: Harness; readonly scanKind: string; readonly driver: string; readonly binary: string; readonly model: string;
  readonly minimumNode?: [number, number];
  probe(cwd: string): Promise<CommandResult>;
  seed(cwd: string, prompt: string): Promise<NativeSeed>;
}
export function parseClaudeSeed(stdout: string): string {
  const parsed = JSON.parse(stdout) as JsonObject;
  if (parsed.is_error !== false || typeof parsed.session_id !== "string" || !parsed.session_id) throw new Error("Claude seed did not return a successful session id");
  return parsed.session_id;
}
export function parseCodexSeed(stdout: string): string {
  const events = stdout.split("\n").filter(Boolean).map((line) => JSON.parse(line) as JsonObject);
  const started = events.find((event) => event.type === "thread.started");
  const agentMessage = events.some((event) => { const item = event.item as JsonObject | undefined; return event.type === "item.completed" && item?.type === "agent_message"; });
  const id = started?.thread_id ?? started?.threadId ?? started?.id;
  if (typeof id !== "string" || !id || !agentMessage) throw new Error("Codex seed did not return a completed thread");
  return id;
}
export function parsePiSeed(stdout: string): string { return parseNdjsonSession(stdout, (event) => event.type === "session" ? event.id : undefined, "Pi"); }
export function parseGrokSeed(stdout: string): string { return parseNdjsonSession(stdout, (event) => event.type === "end" ? event.sessionId : undefined, "Grok"); }
export function parseOpenCodeSeed(stdout: string): string { return parseNdjsonSession(stdout, (event) => event.sessionID, "OpenCode"); }
export function openCodeSeedInvocation(cwd: string, model: string, prompt: string): { args: string[]; env: NodeJS.ProcessEnv } {
  return {
    args: ["run", "--format", "json", "--pure", "--dir", ".", "--model", model, prompt],
    // OpenCode resolves a relative --dir through PWD instead of the spawned
    // process cwd. Keep the native seed in the exact workspace later scanned.
    env: { PWD: cwd },
  };
}
export function groupScenario(runNonce: string, harness: Harness): { one: string; two: string; prompt: string } {
  const one = `Deployment ${runNonce}-${harness} started.`;
  const two = `Deployment ${runNonce}-${harness} completed.`;
  return { one, two, prompt: `Follow the group-chat protocol in AGENTS.md. Publish these two separate project updates to the group, each as its own message: "${one}" and "${two}"` };
}
export function importContinuityScenario(runNonce: string, harness: Harness): { fact: string; requestId: string; seedPrompt: string; followupPrompt: string } {
  const fact = `Juniper-${runNonce.slice(0, 8)}-${harness}`;
  const requestId = `REQ-${runNonce}-${harness}`;
  return {
    fact,
    requestId,
    seedPrompt: `The project codename is ${fact}. Keep that detail available for a later project update, and briefly acknowledge it.`,
    followupPrompt: `Write one concise project update stating the codename established earlier and include request reference ${requestId}.`,
  };
}
function parseNdjsonSession(stdout: string, extract: (event: JsonObject) => unknown, label: string): string {
  let sessionId: string | undefined; let hasText = false;
  for (const line of stdout.split("\n").filter(Boolean)) {
    const event = JSON.parse(line) as JsonObject; const candidate = extract(event);
    if (typeof candidate === "string" && candidate) sessionId = candidate;
    if (event.type === "message_end" || event.type === "text") hasText = true;
  }
  if (!sessionId || !hasText) throw new Error(`${label} seed did not return a completed session`);
  return sessionId;
}
function adaptersFromEnv(): Record<Harness, HarnessAdapter> {
  const claudeBinary = process.env.CHORUZ_CLAUDE_BINARY ?? "claude";
  const codexBinary = process.env.CHORUZ_CODEX_BINARY ?? "codex";
  const claudeModel = process.env.CHORUZ_SMOKE_CLAUDE_MODEL ?? "haiku";
  const codexModel = process.env.CHORUZ_SMOKE_CODEX_MODEL ?? "gpt-5.4-mini";
  const piBinary = process.env.CHORUZ_PI_BINARY ?? "pi"; const piModel = process.env.CHORUZ_SMOKE_PI_MODEL ?? "openrouter/openrouter/free";
  const grokBinary = process.env.CHORUZ_GROK_BINARY ?? "grok"; const grokModel = process.env.CHORUZ_SMOKE_GROK_MODEL ?? "grok-4.6";
  const opencodeBinary = process.env.CHORUZ_OPENCODE_BINARY ?? "opencode"; const opencodeModel = process.env.CHORUZ_SMOKE_OPENCODE_MODEL ?? "opencode/mimo-v2.5-free";
  return {
    claude: { harness: "claude", scanKind: "claude", driver: "claude_terminal", binary: claudeBinary, model: claudeModel,
      probe: (cwd) => runCommand(claudeBinary, ["--version"], { cwd, timeoutMs: 15_000 }),
      seed: async (cwd, prompt) => { const result = await runCommand(claudeBinary, ["-p", "--output-format", "json", "--model", claudeModel], { cwd, input: prompt }); if (result.code !== 0) throw Object.assign(new Error("Claude seed failed"), { commandResult: result }); return { nativeSessionId: parseClaudeSeed(result.stdout), stdout: result.stdout, stderr: result.stderr }; } },
    codex: { harness: "codex", scanKind: "codex", driver: "codex_terminal", binary: codexBinary, model: codexModel,
      probe: (cwd) => runCommand(codexBinary, ["--version"], { cwd, timeoutMs: 15_000 }),
      seed: async (cwd, prompt) => { const result = await runCommand(codexBinary, ["exec", "--json", "--model", codexModel, "--skip-git-repo-check", "-"], { cwd, input: prompt }); if (result.code !== 0) throw Object.assign(new Error("Codex seed failed"), { commandResult: result }); return { nativeSessionId: parseCodexSeed(result.stdout), stdout: result.stdout, stderr: result.stderr }; } },
    pi: { harness: "pi", scanKind: "pi", driver: "pi_terminal", binary: piBinary, model: piModel, minimumNode: [22, 19],
      probe: (cwd) => runCommand(piBinary, ["--version"], { cwd, timeoutMs: 15_000 }),
      seed: async (cwd, prompt) => { const result = await runCommand(piBinary, ["--mode", "json", "--approve", "--no-tools", "--model", piModel, prompt], { cwd }); if (result.code !== 0) throw Object.assign(new Error("Pi seed failed"), { commandResult: result }); return { nativeSessionId: parsePiSeed(result.stdout), stdout: result.stdout, stderr: result.stderr }; } },
    grok: { harness: "grok", scanKind: "grok", driver: "grok_terminal", binary: grokBinary, model: grokModel,
      probe: (cwd) => runCommand(grokBinary, ["--version"], { cwd, timeoutMs: 15_000 }),
      seed: async (cwd, prompt) => { const result = await runCommand(grokBinary, ["--no-auto-update", "-p", prompt, "--output-format", "streaming-json", "--model", grokModel], { cwd }); if (result.code !== 0) throw Object.assign(new Error("Grok seed failed"), { commandResult: result }); return { nativeSessionId: parseGrokSeed(result.stdout), stdout: result.stdout, stderr: result.stderr }; } },
    opencode: { harness: "opencode", scanKind: "open_code", driver: "opencode_terminal", binary: opencodeBinary, model: opencodeModel,
      probe: (cwd) => runCommand(opencodeBinary, ["--version"], { cwd, timeoutMs: 15_000 }),
      seed: async (cwd, prompt) => { const invocation = openCodeSeedInvocation(cwd, opencodeModel, prompt); const result = await runCommand(opencodeBinary, invocation.args, { cwd, env: invocation.env }); if (result.code !== 0) throw Object.assign(new Error("OpenCode seed failed"), { commandResult: result }); return { nativeSessionId: parseOpenCodeSeed(result.stdout), stdout: result.stdout, stderr: result.stderr }; } },
  };
}

class Client {
  readonly apiBase: string;
  readonly webBase: string;
  private token: string;
  constructor(apiBase: string, webBase: string, token = "") { this.apiBase = apiBase; this.webBase = webBase; this.token = token; }
  setToken(token: string): void { this.token = token; }
  private async request(base: string, path: string, init: RequestInit, web = false): Promise<unknown> {
    const headers = new Headers(init.headers);
    if (this.token) headers.set(web ? "cookie" : "authorization", web ? `choruz_session=${this.token}` : `Bearer ${this.token}`);
    const response = await fetch(`${base}${path}`, { ...init, headers, signal: AbortSignal.timeout(15_000) });
    const text = await response.text();
    if (!response.ok) throw new HttpError(response.status, text);
    if (!text) return null;
    try { return JSON.parse(text) as unknown; } catch { throw new Error(`invalid JSON from ${path}`); }
  }
  api(path: string, method = "GET", body?: unknown): Promise<unknown> { return this.request(this.apiBase, path, { method, headers: body === undefined ? undefined : { "content-type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) }); }
  web(path: string, method = "GET", body?: unknown): Promise<unknown> { return this.request(this.webBase, path, { method, headers: body === undefined ? undefined : { "content-type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) }, true); }
}

function object(value: unknown): JsonObject { if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("expected object response"); return value as JsonObject; }
function array(value: unknown): JsonObject[] { if (!Array.isArray(value)) throw new Error("expected array response"); return value.map(object); }
function stringField(value: JsonObject, key: string): string { const field = value[key]; if (typeof field !== "string" || !field) throw new Error(`missing ${key}`); return field; }
function unique(values: string[]): string[] { return [...new Set(values)]; }
function nonce(input: string): string { return createHash("sha256").update(input).digest("hex").slice(0, 12); }
export function hasTrackedResource(entries: JsonObject[], trackedIds: ReadonlySet<string>, companyId: string, namePrefixes: readonly string[]): boolean {
  return entries.some((entry) => {
    const idFields = [entry.id, entry.agent_principal_id, entry.conversation_id];
    const name = entry.name;
    return idFields.some((value) => typeof value === "string" && trackedIds.has(value))
      || entry.workspace_id === companyId
      || (typeof name === "string" && namePrefixes.some((prefix) => name.startsWith(prefix)));
  });
}
export function hasTrackedActiveBinding(entries: JsonObject[], trackedIds: ReadonlySet<string>): boolean {
  return entries.some((entry) => entry.state !== "disabled"
    && [entry.id, entry.agent_principal_id, entry.conversation_id]
      .some((value) => typeof value === "string" && trackedIds.has(value)));
}
async function pollMarker(client: Client, actorId: string, conversationId: string, agentId: string, marker: string): Promise<JsonObject[]> {
  const attempts = Number(process.env.CHORUZ_SMOKE_POLL_ATTEMPTS ?? 90); const delay = Number(process.env.CHORUZ_SMOKE_POLL_SECONDS ?? 2) * 1000;
  for (let attempt = 0; attempt < attempts; attempt += 1) { const messages = array(await client.api(`/v1/conversations/${conversationId}/messages?principal_id=${actorId}`)); if (messages.some((message) => message.sender_id === agentId && typeof message.content === "string" && message.content.includes(marker))) return messages; await new Promise((done) => setTimeout(done, delay)); }
  throw new Error("real reply timeout");
}
async function sendMessage(client: Client, actorId: string, conversationId: string, content: string, key: string): Promise<void> { await client.api("/v1/messages", "POST", { actor_id: actorId, conversation_id: conversationId, content, content_type: "text", metadata: {}, idempotency_key: key }); }
async function bindingFor(client: Client, agentId: string): Promise<JsonObject> { const binding = array(await client.api("/v1/runtime/bindings")).find((entry) => entry.agent_principal_id === agentId); if (!binding) throw new Error("runtime binding missing"); return binding; }
async function classifyAgentFailure(client: Client, agentId: string | undefined, error: unknown, functionalReason: string): Promise<{ verdict: Verdict; reason: string }> {
  const http = classifyHttpError(error); if (http.verdict === "BLOCKED") return http;
  if (agentId) {
    try { const lastError = (await bindingFor(client, agentId)).last_error; if (typeof lastError === "string" && isExplicitHarnessAuthError(lastError)) return { verdict: "BLOCKED", reason: "harness-auth-unavailable" }; }
    catch { /* The original functional failure remains authoritative. */ }
  }
  return { verdict: "FAIL", reason: functionalReason };
}
async function invokeRestartHook(harness: Harness, agentId: string, cwd: string): Promise<void> {
  const hook = process.env.CHORUZ_SMOKE_RESTART_HOOK;
  if (!hook) throw Object.assign(new Error("restart hook not configured"), { blockedReason: "restart-hook-not-configured" });
  if (!isAbsolute(hook)) throw new Error("restart hook must be an absolute executable path");
  await access(hook, fsConstants.X_OK);
  const result = await runCommand(hook, [], { cwd, input: JSON.stringify({ harness, agent_id: agentId }) });
  const response = object(JSON.parse(result.stdout));
  if (result.code !== 0 || response.restarted !== true || typeof response.before_identity !== "string" || typeof response.after_identity !== "string" || !response.before_identity || !response.after_identity || response.before_identity === response.after_identity) throw new Error("restart hook failed");
}
function renderReport(results: Result[], artifactsRemoved: boolean): string {
  const lines = ["Choruz real-Harness platform smoke", "API: configured", "Web: configured"];
  for (const result of results) lines.push(`${result.harness} ${result.scenario}: ${result.verdict} (${result.reason})`);
  lines.push(`Runner artifact directory retained: ${artifactsRemoved ? "no" : "yes"}`, "Credentials retained: no", "Opaque runtime identifiers retained: no", "Harness-native record deletion attempted: no");
  const report = `${lines.join("\n")}\n`;
  if (/(bearer |session[_ -]?id|workspace path|\/Users\/|\/home\/|agt_[a-z0-9]|password|secret)/i.test(report)) throw new Error("sanitized report safety check failed");
  return report;
}

async function main(): Promise<number> {
  const rawHarnesses = (process.env.CHORUZ_REAL_HARNESS_DRIVERS ?? "claude,codex,pi,grok,opencode").split(",").filter(Boolean);
  const unsupported = rawHarnesses.filter((name) => !SUPPORTED_HARNESSES.includes(name as Harness));
  if (unsupported.length) { console.error(`Unsupported live Harness adapter(s): ${unsupported.join(", ")}. Supported: ${SUPPORTED_HARNESSES.join(", ")}.`); return 2; }
  const harnesses = unique(rawHarnesses) as Harness[];
  if (!harnesses.length) { console.error("No Harnesses selected."); return 2; }
  const book = new ResultBook(harnesses);
  const parent = resolve(process.env.CHORUZ_REAL_HARNESS_SMOKE_ROOT_PARENT ?? homedir());
  const root = await mkdtemp(join(parent, "choruz-real-harness.")); const canonicalRoot = await realpath(root);
  const artifactRoot = join(canonicalRoot, "private-artifacts"); const workspaceRoot = join(canonicalRoot, "workspace");
  await mkdir(artifactRoot, { mode: 0o700 }); await mkdir(workspaceRoot, { mode: 0o700 });
  const runNonce = nonce(`${canonicalRoot}-${process.pid}-${Date.now()}`);
  const client = new Client(process.env.CHORUZ_SMOKE_API_BASE_URL ?? "http://127.0.0.1:3000", process.env.CHORUZ_SMOKE_WEB_BASE_URL ?? "http://127.0.0.1:3100");
  const adapters = adaptersFromEnv(); const agents: Provisioned[] = []; const seeded: Seeded[] = []; const directConversationIds: string[] = []; const groupConversationIds: string[] = [];
  let actorId = ""; let companyId = "";
  try {
    try {
      let token = process.env.CHORUZ_SMOKE_SESSION_TOKEN ?? "";
      if (!token) { const username = process.env.CHORUZ_SMOKE_OPERATOR_USER; const password = process.env.CHORUZ_SMOKE_OPERATOR_PASSWORD; if (!username || !password) throw Object.assign(new Error("operator credentials unavailable"), { blockedReason: "operator-auth-unavailable" }); const login = object(await client.api("/v1/auth/local/login", "POST", { username, password })); token = stringField(login, "session_token"); }
      client.setToken(token); const consoleState = object(await client.api("/v1/console")); actorId = stringField(object(consoleState.principal), "id"); book.set("platform", "authentication", "PASS", "operator-auth-confirmed");
    } catch (error) {
      const blocked = (error as { blockedReason?: string }).blockedReason; const classified = blocked ? { verdict: "BLOCKED" as const, reason: blocked } : classifyHttpError(error);
      book.set("platform", "authentication", classified.verdict, classified.reason); for (const harness of harnesses) book.fillHarness(harness, "BLOCKED", "authentication-prerequisite-failed"); book.set("platform", "setup", "BLOCKED", "authentication-prerequisite-failed"); book.set("platform", "cleanup", "PASS", "no-platform-resources-created"); throw new StopRun();
    }
    try { const company = object(await client.api("/v1/companies", "POST", { actor_id: actorId, name: `real-harness-run-${runNonce}`, description: "opt-in real Harness smoke", folder_path: workspaceRoot })); companyId = stringField(company, "id"); book.set("platform", "setup", "PASS", "disposable-company-created"); }
    catch (error) { const classified = classifyHttpError(error); book.set("platform", "setup", classified.verdict, classified.reason); for (const harness of harnesses) book.fillHarness(harness, "BLOCKED", "setup-prerequisite-failed"); book.set("platform", "cleanup", "PASS", "no-platform-resources-created"); throw new StopRun(); }

    for (const harness of harnesses) {
      const adapter = adapters[harness]; const provisionWorkspace = join(workspaceRoot, `provision-${harness}`); const seedWorkspace = join(workspaceRoot, `seed-${harness}`);
      await mkdir(provisionWorkspace, { mode: 0o700 }); await mkdir(seedWorkspace, { mode: 0o700 });
      if (adapter.minimumNode) {
        const [major, minor] = process.versions.node.split(".").map(Number); const [requiredMajor, requiredMinor] = adapter.minimumNode;
        if (major < requiredMajor || (major === requiredMajor && minor < requiredMinor)) { book.fillHarness(harness, "BLOCKED", "node-version-unavailable"); continue; }
      }
      if (!adapter.model) { book.fillHarness(harness, "BLOCKED", "model-not-configured"); continue; }
      try { const probe = await adapter.probe(provisionWorkspace); if (probe.code !== 0) throw Object.assign(new Error("Harness probe failed"), { commandResult: probe }); }
      catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") book.fillHarness(harness, "SKIP", "harness-unavailable"); else book.fillHarness(harness, "FAIL", "harness-probe-failed"); continue; }
      try {
        const models = object(await client.web(`/api/drivers/models?driver_type=${adapter.driver}`));
        if (models.status === "auth_required") { book.set(harness, "model-discovery", "BLOCKED", "harness-auth-unavailable"); book.fillHarness(harness, "BLOCKED", "model-prerequisite-failed"); continue; }
        const available = Array.isArray(models.models) && models.status === "available" && models.models.some((entry) => { const model = object(entry); return model.id === adapter.model || model.resolvedModel === adapter.model; });
        if (!available) throw new Error("configured model was not discovered"); book.set(harness, "model-discovery", "PASS", "configured-model-confirmed");
      } catch (error) { const classified = classifyHttpError(error); book.set(harness, "model-discovery", classified.verdict, classified.reason === "platform-request-failed" ? "model-discovery-failed" : classified.reason); book.fillHarness(harness, "BLOCKED", "model-prerequisite-failed"); continue; }
      let provisioned: Provisioned | undefined; const markerOne = `DM_ONE_${runNonce}_${harness}`;
      try {
        const provision = object(await client.web("/api/agents/provision", "POST", { name: `smoke-run-${runNonce}-${harness}`, driver_type: adapter.driver, model: adapter.model, workspace_id: companyId, workspace_path: provisionWorkspace, instructions: "Real acceptance agent. Direct chat: echo requested markers. Group chat: use only the bound absolute CHORUZ_SEND helper, invoking it separately for each requested message.", idempotency_key: `real-harness-smoke-${runNonce}-${harness}` }));
        provisioned = { harness, agentId: stringField(object(provision.agent), "id"), conversationId: stringField(object(provision.conversation), "id") }; agents.push(provisioned); directConversationIds.push(provisioned.conversationId);
        await sendMessage(client, actorId, provisioned.conversationId, `Reply with ${markerOne}.`, `${runNonce}-${harness}-dm-one`); await pollMarker(client, actorId, provisioned.conversationId, provisioned.agentId, markerOne); provisioned.sessionId = stringField(await bindingFor(client, provisioned.agentId), "external_session_id"); book.set(harness, "provision-dm", "PASS", "real-cli-and-session-confirmed");
      } catch (error) { const classified = await classifyAgentFailure(client, provisioned?.agentId, error, "real-dm-flow-failed"); book.set(harness, "provision-dm", classified.verdict, classified.reason); book.fillHarness(harness, "BLOCKED", "provision-prerequisite-failed"); continue; }
      if (!provisioned) { book.set(harness, "provision-dm", "FAIL", "invalid-provision-response"); book.fillHarness(harness, "BLOCKED", "provision-prerequisite-failed"); continue; }
      try {
        await invokeRestartHook(harness, provisioned.agentId, provisionWorkspace); const markerTwo = `DM_TWO_${runNonce}_${harness}`;
        await sendMessage(client, actorId, provisioned.conversationId, `Quote the marker from the immediately previous turn, then append ${markerTwo}.`, `${runNonce}-${harness}-dm-two`);
        const messages = await pollMarker(client, actorId, provisioned.conversationId, provisioned.agentId, markerTwo); const continued = messages.some((message) => message.sender_id === provisioned.agentId && typeof message.content === "string" && message.content.includes(markerOne) && message.content.includes(markerTwo)); const sessionAfter = stringField(await bindingFor(client, provisioned.agentId), "external_session_id");
        if (!continued || sessionAfter !== provisioned.sessionId) throw new Error("session did not survive restart"); book.set(harness, "restart-resume", "PASS", "real-process-restart-resumed");
      } catch (error) { const blocked = (error as { blockedReason?: string }).blockedReason; const classified = blocked ? { verdict: "BLOCKED" as const, reason: blocked } : await classifyAgentFailure(client, provisioned.agentId, error, "restart-resume-failed"); book.set(harness, "restart-resume", classified.verdict, classified.reason); }
      try {
        const scenario = importContinuityScenario(runNonce, harness); const seed = await adapter.seed(seedWorkspace, scenario.seedPrompt);
        await writeFile(join(artifactRoot, `${harness}-seed.stdout`), seed.stdout, { mode: 0o600 }); await writeFile(join(artifactRoot, `${harness}-seed.stderr`), seed.stderr, { mode: 0o600 }); seeded.push({ harness, workspace: seedWorkspace, marker: scenario.fact, nativeSessionId: seed.nativeSessionId });
      } catch (error) { const result = (error as { commandResult?: CommandResult }).commandResult; const auth = result && isExplicitHarnessAuthError(result.stderr); book.set(harness, "scan-import-context", auth ? "BLOCKED" : "FAIL", auth ? "harness-auth-unavailable" : "native-seed-failed"); }
    }

    if (agents.length) {
      let groupId: string | undefined; let groupReady = false;
      try { const group = object(await client.api("/v1/groups", "POST", { actor_id: actorId, name: `smoke-group-${runNonce}`, description: "real Harness multi-send smoke", avatar_url: null, member_ids: agents.map((agent) => agent.agentId), workspace_id: companyId })); groupId = stringField(group, "id"); groupConversationIds.push(groupId); const members = object(group.members); const expectedMembers = [actorId, ...agents.map((agent) => agent.agentId)]; if (group.workspace_id !== companyId || group.conversation_type !== "group" || group.name !== `smoke-group-${runNonce}` || !expectedMembers.every((id) => id in members)) throw new Error("group ledger mismatch"); groupReady = true; }
      catch (error) { const classified = classifyHttpError(error); for (const agent of agents) book.set(agent.harness, "group-two-helper-sends", classified.verdict, classified.verdict === "BLOCKED" ? classified.reason : "group-create-failed"); }
      if (groupId && groupReady) for (const agent of agents) {
        try { const scenario = groupScenario(runNonce, agent.harness); await sendMessage(client, actorId, groupId, `@smoke-run-${runNonce}-${agent.harness} ${scenario.prompt}`, `${runNonce}-${agent.harness}-group`); const messages = await pollMarker(client, actorId, groupId, agent.agentId, scenario.two); const exact = (content: string) => messages.filter((message) => message.sender_id === agent.agentId && message.content === content).length; if (exact(scenario.one) !== 1 || exact(scenario.two) !== 1) throw new Error("helper send count mismatch"); book.set(agent.harness, "group-two-helper-sends", "PASS", "real-helper-confirmed"); }
        catch (error) { const classified = await classifyAgentFailure(client, agent.agentId, error, "group-flow-failed"); book.set(agent.harness, "group-two-helper-sends", classified.verdict, classified.reason); }
      }
    }
    if (seeded.length) {
      let scan: JsonObject[] = [];
      try { const response = object(await client.api("/v1/workspace-sessions/scan", "POST", { workspace_path: workspaceRoot, harnesses: seeded.map((seed) => adapters[seed.harness].scanKind) })); scan = array(response.sessions); }
      catch { for (const seed of seeded) book.set(seed.harness, "scan-import-context", "FAIL", "recursive-scan-failed"); }
      for (const seed of seeded) {
        let importedAgentId: string | undefined;
        try {
          const scanKind = adapters[seed.harness].scanKind; const matches = scan.filter((session) => session.harness === scanKind && session.workspace_path === seed.workspace && session.native_session_id === seed.nativeSessionId); if (matches.length !== 1) throw new Error("exact seeded session not uniquely discovered");
          const imported = object(await client.api("/v1/workspace-sessions/import", "POST", { company_id: companyId, workspace_path: workspaceRoot, sessions: [{ harness: scanKind, native_session_id: seed.nativeSessionId, workspace_path: seed.workspace }] })); const importedSessions = array(imported.imported); if (importedSessions.length !== 1 || importedSessions[0].native_session_id !== seed.nativeSessionId) throw new Error("import response session id mismatch");
          const agentId = stringField(importedSessions[0], "agent_principal_id"); importedAgentId = agentId; const conversationId = stringField(importedSessions[0], "conversation_id"); agents.push({ harness: seed.harness, agentId, conversationId }); directConversationIds.push(conversationId); if ((await bindingFor(client, agentId)).external_session_id !== seed.nativeSessionId) throw new Error("binding session id mismatch");
          const scenario = importContinuityScenario(runNonce, seed.harness); await sendMessage(client, actorId, conversationId, scenario.followupPrompt, `${runNonce}-${seed.harness}-import`); const messages = await pollMarker(client, actorId, conversationId, agentId, scenario.requestId); const resumed = messages.some((message) => message.sender_id === agentId && typeof message.content === "string" && message.content.includes(seed.marker) && message.content.includes(scenario.requestId)); if (!resumed) throw new Error("imported context not resumed"); book.set(seed.harness, "scan-import-context", "PASS", "exact-native-session-resumed");
        } catch (error) { const classified = await classifyAgentFailure(client, importedAgentId, error, "exact-import-resume-failed"); book.set(seed.harness, "scan-import-context", classified.verdict, classified.reason); }
      }
    }
  } catch (error) {
    if (!(error instanceof StopRun)) book.set("platform", "setup", "FAIL", "runner-internal-failure");
  } finally {
    if (actorId && companyId) {
      let cleanupFailed = false;
      const agentIds = unique(agents.map((agent) => agent.agentId)); const groups = unique(groupConversationIds); const directConversations = unique(directConversationIds); const allConversations = unique([...groups, ...directConversations]);
      if (groups.length) try { const cleanup = object(await client.api("/v1/agents/batch-disable", "POST", { actor_id: actorId, agent_ids: [], conversation_ids: groups })); if (cleanup.disabled !== 0 || cleanup.failed !== 0 || cleanup.conversations_deleted !== groups.length || cleanup.conversations_failed !== 0) cleanupFailed = true; }
      catch { cleanupFailed = true; }
      try { const cleanup = object(await client.api("/v1/agents/batch-disable", "POST", { actor_id: actorId, agent_ids: agentIds, conversation_ids: [] })); if (cleanup.disabled !== agentIds.length || cleanup.failed !== 0 || cleanup.conversations_deleted !== 0 || cleanup.conversations_failed !== 0) cleanupFailed = true; }
      catch { cleanupFailed = true; }
      try {
        await client.api(`/v1/companies/${companyId}`, "DELETE");
        const companies = array(await client.api("/v1/companies")); const consoleState = object(await client.api("/v1/console")); const conversations = array(await client.api(`/v1/conversations?principal_id=${actorId}`)); const bindings = array(await client.api("/v1/runtime/bindings"));
        const trackedIds = new Set([companyId, ...agentIds, ...allConversations]); const prefixes = [`real-harness-run-${runNonce}`, `smoke-run-${runNonce}-`, `smoke-group-${runNonce}`];
        const consoleEntries = [...array(consoleState.agents), ...array(consoleState.principals), ...array(consoleState.conversations)];
        if (hasTrackedResource(companies, trackedIds, companyId, prefixes) || hasTrackedResource(consoleEntries, trackedIds, companyId, prefixes) || hasTrackedResource(conversations, trackedIds, companyId, prefixes) || hasTrackedActiveBinding(bindings, trackedIds)) cleanupFailed = true;
      }
      catch { cleanupFailed = true; }
      book.set("platform", "cleanup", cleanupFailed ? "FAIL" : "PASS", cleanupFailed ? "platform-cleanup-failed" : "platform-resources-verified-removed");
    }
  }
  const canonicalParent = await realpath(parent); if (!canonicalRoot.startsWith(`${canonicalParent}/`) || !basename(canonicalRoot).startsWith("choruz-real-harness.")) throw new Error("refusing unsafe artifact cleanup");
  let artifactsRemoved = true; try { await rm(canonicalRoot, { recursive: true, force: false }); } catch { artifactsRemoved = false; book.set("platform", "cleanup", "FAIL", "artifact-cleanup-failed"); }
  const report = renderReport(book.entries(), artifactsRemoved); process.stdout.write(report); const reportPath = process.env.CHORUZ_REAL_HARNESS_SMOKE_REPORT;
  if (reportPath) { const destination = resolve(reportPath); if (destination.startsWith(`${canonicalRoot}/`)) throw new Error("report path must be outside disposable root"); await writeFile(destination, report, { flag: "wx", mode: 0o600 }); }
  return book.exitCode();
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) main().then((code) => { process.exitCode = code; }).catch((error: unknown) => { console.error(error instanceof Error ? error.message : String(error)); process.exitCode = 1; });
