import { spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { promises as fs } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";

import { query, type ModelInfo } from "@anthropic-ai/claude-agent-sdk";

import { resolveDriverBinary } from "../drivers/driver-availability";
import { postgresQueryClient, withPostgresTransaction } from "../groups/group-provisioning-db";
import type { DriverModel } from "../drivers/driver-models";
import { canonicalUsageLabel } from "./harness-account-display";

export type AccountDriver = "claude_terminal" | "codex_terminal";
export type HarnessAccountStatus = "pending" | "active" | "reauth_required" | "error" | "disabled";
export type HarnessAccountProfileKind = "default" | "isolated";

export type UsageWindow = {
  id: string;
  label: string;
  usedPercent: number;
  remainingPercent: number;
  resetsAt: string | null;
  windowDurationMinutes: number | null;
};

export type HarnessAccount = {
  id: string;
  companyId: string;
  runtimeHostId: string | null;
  driverType: AccountDriver;
  name: string;
  profileKind: HarnessAccountProfileKind;
  subscriptionType: string | null;
  status: HarnessAccountStatus;
  models: DriverModel[];
  usage: { windows: UsageWindow[] };
  lastError: string | null;
  probedAt: string | null;
  createdAt: string;
  updatedAt: string;
};

type AccountRow = {
  id: string;
  company_id: string;
  runtime_host_id: string | null;
  driver_type: AccountDriver;
  name: string;
  profile_kind: HarnessAccountProfileKind;
  account_fingerprint: string | null;
  subscription_type: string | null;
  status: HarnessAccountStatus;
  models_json: DriverModel[];
  usage_json: { windows?: UsageWindow[] };
  last_error: string | null;
  probed_at: Date | string | null;
  created_at: Date | string;
  updated_at: Date | string;
};

const ACCOUNT_COLUMNS = `id, company_id, runtime_host_id, driver_type, name, profile_kind,
            account_fingerprint, subscription_type, status, models_json, usage_json,
            last_error, probed_at, created_at, updated_at`;

export function harnessAccountRoot(env: NodeJS.ProcessEnv = process.env): string {
  return path.resolve(env.CHORUZ_HARNESS_ACCOUNT_ROOT?.trim() || path.join(homedir(), ".choruz", "accounts"));
}

export function harnessAccountProfileDir(account: Pick<HarnessAccount, "id" | "driverType" | "profileKind">): string | null {
  if (account.profileKind === "default") return null;
  if (!/^[0-9a-f-]{36}$/i.test(account.id)) throw new Error("Invalid harness account id");
  return path.join(harnessAccountRoot(), account.id, account.driverType === "claude_terminal" ? "claude" : "codex");
}

export function harnessAccountEnv(account: Pick<HarnessAccount, "id" | "driverType" | "profileKind">): Record<string, string> {
  const profileDir = harnessAccountProfileDir(account);
  if (!profileDir) return {};
  return account.driverType === "claude_terminal"
    ? { CLAUDE_CONFIG_DIR: profileDir }
    : { CODEX_HOME: profileDir };
}

export async function listHarnessAccounts(companyId: string, runtimeHostId: string | null): Promise<HarnessAccount[]> {
  const client = await postgresQueryClient();
  const result = await client.query<AccountRow>(
    `SELECT ${ACCOUNT_COLUMNS}
       FROM harness_account
      WHERE company_id = $1 AND runtime_host_id IS NOT DISTINCT FROM $2
        AND disabled_at IS NULL
      ORDER BY lower(name), id`,
    [companyId, runtimeHostId],
  );
  return result.rows.map(mapAccount);
}

export async function getHarnessAccount(id: string, companyId: string): Promise<HarnessAccount | null> {
  const client = await postgresQueryClient();
  const result = await client.query<AccountRow>(
    `SELECT ${ACCOUNT_COLUMNS}
       FROM harness_account
      WHERE id = $1 AND company_id = $2 AND disabled_at IS NULL`,
    [id, companyId],
  );
  return result.rows[0] ? mapAccount(result.rows[0]) : null;
}

export async function runtimeHostBelongsToCompany(runtimeHostId: string, companyId: string): Promise<boolean> {
  const client = await postgresQueryClient();
  const result = await client.query(
    `SELECT 1 FROM runtime_host
      WHERE id = $1 AND company_id = $2 AND revoked_at IS NULL`,
    [runtimeHostId, companyId],
  );
  return result.rowCount === 1;
}

export async function disableHarnessAccount(id: string, companyId: string): Promise<number> {
  return withPostgresTransaction(async (client) => {
    const account = await client.query<{ id: string }>(
      `SELECT id FROM harness_account
        WHERE id = $1 AND company_id = $2 AND disabled_at IS NULL
        FOR UPDATE`,
      [id, companyId],
    );
    if (!account.rows[0]) throw new Error("Harness account not found");
    const disabled = await client.query(
      `UPDATE agent_runtime_bindings AS binding
          SET state = 'disabled', in_flight_turn_id = NULL, updated_at = NOW()
         FROM principal
        WHERE principal.id = binding.agent_principal_id
          AND principal.workspace_id = $2
          AND binding.state <> 'disabled'
          AND binding.config_json->>'harness_account_id' = $1`,
      [id, companyId],
    );
    await client.query(
      `UPDATE harness_account
          SET status = 'disabled', disabled_at = NOW(), updated_at = NOW()
        WHERE id = $1 AND company_id = $2 AND disabled_at IS NULL`,
      [id, companyId],
    );
    return disabled.rowCount ?? 0;
  });
}

export async function createHarnessAccount(input: {
  companyId: string;
  runtimeHostId: string | null;
  driverType: AccountDriver;
  name: string;
  profileKind: HarnessAccountProfileKind;
}): Promise<HarnessAccount> {
  const id = randomUUID();
  let accountDir: string | null = null;
  if (input.profileKind === "isolated" && !input.runtimeHostId) {
    const profileDir = harnessAccountProfileDir({ id, driverType: input.driverType, profileKind: input.profileKind });
    if (!profileDir) throw new Error("Unable to resolve isolated harness account profile");
    accountDir = path.dirname(profileDir);
    await fs.mkdir(profileDir, { recursive: true, mode: 0o700 });
    await fs.chmod(profileDir, 0o700);
  }
  const client = await postgresQueryClient();
  try {
    const result = await client.query<AccountRow>(
      `INSERT INTO harness_account
         (id, company_id, runtime_host_id, driver_type, name, profile_kind)
       VALUES ($1, $2, $3, $4, $5, $6)
       RETURNING ${ACCOUNT_COLUMNS}`,
      [id, input.companyId, input.runtimeHostId, input.driverType, input.name.trim(), input.profileKind],
    );
    return mapAccount(result.rows[0]);
  } catch (error) {
    if (accountDir) await fs.rm(accountDir, { recursive: true, force: true });
    throw error;
  }
}

/** The label of the account Choruz registers for the login a device already has. */
export function defaultHarnessAccountName(driverType: AccountDriver): string {
  return driverType === "claude_terminal" ? "Claude Code login" : "Codex login";
}

/**
 * The `default` account of one device and harness: the login the device
 * already has, registered by Choruz the first time anything needs it.
 */
export async function ensureDefaultHarnessAccount(input: {
  companyId: string;
  runtimeHostId: string | null;
  driverType: AccountDriver;
}): Promise<HarnessAccount> {
  const client = await postgresQueryClient();
  const params = [input.companyId, input.runtimeHostId, input.driverType];
  const find = () => client.query<AccountRow>(
    `SELECT ${ACCOUNT_COLUMNS}
       FROM harness_account
      WHERE company_id = $1 AND runtime_host_id IS NOT DISTINCT FROM $2
        AND driver_type = $3 AND profile_kind = 'default' AND disabled_at IS NULL`,
    params,
  );
  const existing = await find();
  if (existing.rows[0]) return mapAccount(existing.rows[0]);
  const inserted = await client.query<AccountRow>(
    `INSERT INTO harness_account
       (id, company_id, runtime_host_id, driver_type, name, profile_kind)
     VALUES ($1, $2, $3, $4, $5, 'default')
     ON CONFLICT DO NOTHING
     RETURNING ${ACCOUNT_COLUMNS}`,
    [randomUUID(), ...params, defaultHarnessAccountName(input.driverType)],
  );
  if (inserted.rows[0]) return mapAccount(inserted.rows[0]);
  const raced = await find();
  if (!raced.rows[0]) throw new Error("Unable to register the device's default harness account");
  return mapAccount(raced.rows[0]);
}

/** Whether a user explicitly removed this device-and-harness default account. */
export async function defaultHarnessAccountWasRemoved(input: {
  companyId: string;
  runtimeHostId: string | null;
  driverType: AccountDriver;
}): Promise<boolean> {
  const client = await postgresQueryClient();
  const result = await client.query<{ removed: boolean }>(
    `SELECT EXISTS (
       SELECT 1 FROM harness_account
        WHERE company_id = $1 AND runtime_host_id IS NOT DISTINCT FROM $2
          AND driver_type = $3 AND profile_kind = 'default' AND disabled_at IS NOT NULL
     ) AND NOT EXISTS (
       SELECT 1 FROM harness_account
        WHERE company_id = $1 AND runtime_host_id IS NOT DISTINCT FROM $2
          AND driver_type = $3 AND profile_kind = 'default' AND disabled_at IS NULL
     ) AS removed`,
    [input.companyId, input.runtimeHostId, input.driverType],
  );
  return result.rows[0]?.removed ?? false;
}

/**
 * The account an agent launches under when none was chosen: the device's
 * default account once it verifies, else none (the agent inherits the
 * device's login without quota or model data). A local account that has
 * not verified yet is probed here; a remote one verifies through its own
 * sign-in.
 */
export async function defaultHarnessAccountForLaunch(input: {
  companyId: string;
  runtimeHostId: string | null;
  driverType: AccountDriver;
  model?: string;
}): Promise<HarnessAccount | null> {
  let account = await ensureDefaultHarnessAccount(input);
  if (account.status !== "active" && !account.runtimeHostId) {
    try {
      account = await probeHarnessAccount(account);
    } catch {
      return null;
    }
  }
  if (account.status !== "active") return null;
  if (input.model && !account.models.some((model) => model.id === input.model)) return null;
  return account;
}

export async function probeHarnessAccount(account: HarnessAccount): Promise<HarnessAccount> {
  if (account.runtimeHostId) throw new Error("Remote account probing must run on its runtime host");
  const env = { ...process.env, ...harnessAccountEnv(account) } as Record<string, string>;
  const binary = resolveDriverBinary(account.driverType, env);
  if (!binary) throw new Error("Harness binary is not configured on this device");
  try {
    const probe = account.driverType === "claude_terminal"
      ? await probeClaude(binary, env)
      : await probeCodex(binary, env);
    if (!probe.models.length) throw new Error("Harness returned no selectable models for this account");
    const client = await postgresQueryClient();
    const result = await client.query<AccountRow>(
      `UPDATE harness_account
          SET account_fingerprint = $2, subscription_type = $3, status = 'active',
              models_json = $4::jsonb, usage_json = $5::jsonb, last_error = NULL,
              probed_at = NOW(), updated_at = NOW()
        WHERE id = $1 AND disabled_at IS NULL
      RETURNING ${ACCOUNT_COLUMNS}`,
      [account.id, accountFingerprint(probe.identifier), probe.subscriptionType, JSON.stringify(probe.models), JSON.stringify({ windows: probe.windows })],
    );
    if (!result.rows[0]) throw new Error("Harness account no longer exists");
    return mapAccount(result.rows[0]);
  } catch (error) {
    const message = safeProbeError(error);
    const status: HarnessAccountStatus = /auth|login|credential|unauthorized/i.test(message)
      ? "reauth_required"
      : "error";
    const client = await postgresQueryClient();
    await client.query(
      `UPDATE harness_account SET status = $2, last_error = $3, updated_at = NOW()
        WHERE id = $1 AND disabled_at IS NULL`,
      [account.id, status, message],
    );
    throw new Error(message);
  }
}

type ProbeResult = {
  identifier: string;
  subscriptionType: string | null;
  models: DriverModel[];
  windows: UsageWindow[];
};

async function probeClaude(binary: string, env: Record<string, string>): Promise<ProbeResult> {
  let finishInput: ((result: IteratorResult<never>) => void) | undefined;
  const input: AsyncIterable<never> & AsyncIterator<never> = {
    [Symbol.asyncIterator]() { return this; },
    next() { return new Promise((resolve) => { finishInput = resolve; }); },
    return() {
      finishInput?.({ done: true, value: undefined as never });
      return Promise.resolve({ done: true, value: undefined as never });
    },
  };
  const session = query({
    prompt: input,
    options: {
      pathToClaudeCodeExecutable: binary,
      cwd: process.cwd(),
      env,
      settingSources: ["user", "project", "local"],
    },
  });
  try {
    const [account, models, usage] = await Promise.all([
      withTimeout(session.accountInfo(), 20_000),
      withTimeout(session.supportedModels(), 20_000),
      withTimeout(session.usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET(), 20_000),
    ]);
    if (!account.email && !account.organization) throw new Error("Claude account identity is unavailable; run /login for this profile");
    if (!usage.rate_limits_available || !usage.rate_limits) throw new Error("Claude did not return exact plan rate limits for this account");
    const windows = claudeUsageWindows(usage.rate_limits);
    if (!windows.length) throw new Error("Claude returned no exact rate-limit windows for this account");
    return {
      identifier: account.email || account.organization || "Claude account",
      subscriptionType: usage.subscription_type || account.subscriptionType || null,
      models: models.map(claudeModel),
      windows,
    };
  } finally {
    await input.return?.();
    session.close();
  }
}

export function claudeUsageWindows(rateLimits: unknown): UsageWindow[] {
  const rawLimits = asRecord(rateLimits);
  return Object.entries(rawLimits).flatMap(([id, raw]) => {
      if (id === "model_scoped" && Array.isArray(raw)) {
        return raw.flatMap((entry) => {
          const value = asRecord(entry);
          return typeof value.utilization === "number"
            ? [usageWindow(
                `${id}:${String(value.display_name ?? "model")}`,
                String(value.display_name ?? "model weekly"),
                value.utilization,
                typeof value.resets_at === "string" ? value.resets_at : null,
                null,
              )]
            : [];
        });
      }
      const value = asRecord(raw);
      if (typeof value.utilization !== "number") return [];
      return [usageWindow(
        id,
        canonicalUsageLabel("claude_terminal", id, id.replaceAll("_", " ")),
        value.utilization,
        typeof value.resets_at === "string" ? value.resets_at : null,
        null,
      )];
  });
}

export async function probeCodex(binary: string, env: Record<string, string>): Promise<ProbeResult> {
  const results = await codexRequests(binary, env, [
    { id: 1, method: "account/read", params: { refreshToken: true } },
    { id: 2, method: "account/rateLimits/read" },
    { id: 3, method: "model/list", params: { limit: 100, includeHidden: false } },
  ]);
  return parseCodexProbeResults(results);
}

export function parseCodexProbeResults(results: Map<number, unknown>): ProbeResult {
  const accountResult = asRecord(results.get(1));
  const account = asRecord(accountResult.account);
  if (!Object.keys(account).length) throw new Error("Codex account is unavailable; run codex login for this profile");
  const identifier = typeof account.email === "string"
    ? account.email
    : typeof account.type === "string" ? `${account.type} account` : "Codex account";
  const rateResult = asRecord(results.get(2));
  const snapshots: Array<[string, unknown]> = rateResult.rateLimitsByLimitId && typeof rateResult.rateLimitsByLimitId === "object"
    ? Object.entries(rateResult.rateLimitsByLimitId as Record<string, unknown>)
    : [["default", rateResult.rateLimits]];
  const seenWindows = new Set<string>();
  const windows = snapshots.flatMap(([limitId, raw]) => {
    const snapshot = asRecord(raw);
    const limitName = typeof snapshot.limitName === "string" && snapshot.limitName.trim()
      ? snapshot.limitName.trim()
      : limitId === "codex" || limitId === "default"
        ? ""
        : limitId.replaceAll("_", " ");
    return (["primary", "secondary"] as const).flatMap((kind) => {
      const window = asRecord(snapshot[kind]);
      if (typeof window.usedPercent !== "number") return [];
      const duration = typeof window.windowDurationMins === "number" ? window.windowDurationMins : null;
      const reset = typeof window.resetsAt === "number" ? new Date(window.resetsAt * 1000).toISOString() : null;
      const signature = `${duration ?? "unknown"}:${reset ?? "unknown"}:${window.usedPercent}`;
      if (seenWindows.has(signature)) return [];
      seenWindows.add(signature);
      const period = duration === 10_080 ? "Weekly" : duration === 300 ? "5-hour" : kind;
      return [usageWindow(
        `${limitId}:${kind}`,
        limitName ? `${limitName} ${period}` : period,
        window.usedPercent,
        reset,
        duration,
      )];
    });
  });
  if (!windows.length) throw new Error("Codex returned no exact rate-limit windows for this account");
  const modelResult = asRecord(results.get(3));
  const models = Array.isArray(modelResult.data) ? modelResult.data : [];
  return {
    identifier,
    subscriptionType: typeof account.planType === "string" ? account.planType : null,
    models: models.flatMap((raw) => {
      const model = asRecord(raw);
      if (typeof model.id !== "string") return [];
      return [{ id: model.id, label: typeof model.displayName === "string" ? model.displayName : model.id }];
    }),
    windows,
  };
}

function codexRequests(binary: string, env: Record<string, string>, requests: Array<Record<string, unknown>>): Promise<Map<number, unknown>> {
  return new Promise((resolve, reject) => {
    const nodeEnv = env.NODE_ENV === "development" || env.NODE_ENV === "test"
      ? env.NODE_ENV
      : "production";
    const child = spawn(binary, ["app-server"], {
      stdio: ["pipe", "pipe", "pipe"] as const,
      env: {
        ...env,
        NODE_ENV: nodeEnv,
        CODEX_DISABLE_UPDATE_CHECK: "1",
      },
    });
    const results = new Map<number, unknown>();
    let stdout = "";
    let stderr = "";
    let settled = false;
    let killTimer: ReturnType<typeof setTimeout> | undefined;
    const timer = setTimeout(() => finish(new Error("Codex account probe timed out")), 25_000);
    const finish = (error?: Error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      child.stdin.destroy();
      if (child.exitCode === null && child.signalCode === null) {
        child.kill("SIGTERM");
        killTimer = setTimeout(() => {
          if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
        }, 1_000);
        killTimer.unref?.();
      }
      error ? reject(error) : resolve(results);
    };
    const send = (message: Record<string, unknown>) => child.stdin.write(
      `${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`,
      (error) => error && finish(error),
    );
    const consume = (line: string) => {
      if (!line.trim()) return;
      let message: Record<string, unknown>;
      try { message = JSON.parse(line) as Record<string, unknown>; } catch { return; }
      if (message.id === 0) {
        if (message.error) return finish(new Error("Codex app-server initialization failed"));
        send({ method: "initialized" });
        requests.forEach(send);
        return;
      }
      if (typeof message.id !== "number" || message.id <= 0) return;
      if (message.error) return finish(new Error(`Codex ${requests.find((item) => item.id === message.id)?.method ?? "request"} failed`));
      results.set(message.id, message.result);
      if (results.size === requests.length) finish();
    };
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      stdout += chunk;
      const lines = stdout.split(/\r?\n/);
      stdout = lines.pop() ?? "";
      lines.forEach(consume);
    });
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => { stderr = `${stderr}${chunk}`.slice(-4096); });
    child.stdin.on("error", finish);
    child.on("error", finish);
    child.on("exit", () => {
      if (killTimer) clearTimeout(killTimer);
      if (!settled) finish(new Error(stderr || "Codex app-server exited during account probe"));
    });
    send({
      method: "initialize",
      id: 0,
      params: {
        clientInfo: { name: "choruz", title: "Choruz", version: "1.0.0" },
        capabilities: { experimentalApi: true },
      },
    });
  });
}

function claudeModel(model: ModelInfo): DriverModel {
  return { id: model.value, label: model.displayName, description: model.description, ...(model.resolvedModel ? { resolvedModel: model.resolvedModel } : {}) };
}

function usageWindow(id: string, label: string, usedPercent: number, resetsAt: string | null, duration: number | null): UsageWindow {
  const bounded = Math.max(0, Math.min(100, usedPercent));
  return { id, label, usedPercent: bounded, remainingPercent: 100 - bounded, resetsAt, windowDurationMinutes: duration };
}

function mapAccount(row: AccountRow): HarnessAccount {
  const date = (value: Date | string | null) => value == null ? null : new Date(value).toISOString();
  return {
    id: row.id,
    companyId: row.company_id,
    runtimeHostId: row.runtime_host_id,
    driverType: row.driver_type,
    name: row.name,
    profileKind: row.profile_kind,
    subscriptionType: row.subscription_type,
    status: row.status,
    models: Array.isArray(row.models_json) ? row.models_json : [],
    usage: { windows: Array.isArray(row.usage_json?.windows) ? row.usage_json.windows : [] },
    lastError: row.last_error,
    probedAt: date(row.probed_at),
    createdAt: date(row.created_at)!,
    updatedAt: date(row.updated_at)!,
  };
}

function accountFingerprint(identifier: string): string {
  return createHash("sha256").update(identifier.trim().toLowerCase()).digest("hex");
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function safeProbeError(error: unknown): string {
  const message = error instanceof Error ? error.message : "";
  if (/no selectable models/i.test(message)) return "This account returned no selectable models; sign in again and verify it";
  if (/auth|login|credential|unauthorized/i.test(message)) return "Harness login is invalid; sign in to this profile and verify again";
  if (/timed out/i.test(message)) return "Harness account probe timed out";
  if (/binary|ENOENT|not configured/i.test(message)) return "Harness binary is not configured on this device";
  if (/no exact rate-limit|exact plan rate limits/i.test(message)) return "Harness did not return exact quota data for this account";
  if (/identity is unavailable|account is unavailable/i.test(message)) return "Harness account identity is unavailable; sign in and verify again";
  return "Harness account probe failed";
}

async function withTimeout<T>(promise: Promise<T>, timeoutMs: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<T>((_, reject) => { timer = setTimeout(() => reject(new Error("Harness account probe timed out")), timeoutMs); }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}
