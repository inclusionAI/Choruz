import { randomUUID } from "node:crypto";
import { promises as fs } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";

import { apiBaseUrl } from "../api/choruz-api";
import { postgresQueryClient, withPostgresTransaction } from "../groups/group-provisioning-db";
import type { DriverModel } from "../drivers/driver-models";

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
 * device's login without quota or model data). An account that has not
 * verified yet is probed here on the device that holds its login.
 */
export async function defaultHarnessAccountForLaunch(sessionToken: string, input: {
  companyId: string;
  runtimeHostId: string | null;
  driverType: AccountDriver;
  model?: string;
}): Promise<HarnessAccount | null> {
  let account = await ensureDefaultHarnessAccount(input);
  if (account.status !== "active") {
    try {
      account = await probeHarnessAccount(sessionToken, account);
    } catch {
      return null;
    }
  }
  if (account.status !== "active") return null;
  if (input.model && !account.models.some((model) => model.id === input.model)) return null;
  return account;
}

/**
 * Read the account's identity, models and exact quota on the device that
 * holds its login, then return the stored account. The gateway runs the
 * probe on its own device or over the runtime host's link and records a
 * failure on the account before answering 409.
 */
export async function probeHarnessAccount(sessionToken: string, account: HarnessAccount): Promise<HarnessAccount> {
  const response = await fetch(
    `${apiBaseUrl()}/v1/companies/${encodeURIComponent(account.companyId)}/harness-accounts/${encodeURIComponent(account.id)}/probe`,
    { method: "POST", headers: { Authorization: `Bearer ${sessionToken}` }, cache: "no-store" },
  );
  if (!response.ok) {
    let message = "Harness account probe failed";
    try {
      const body = (await response.json()) as { error?: string };
      if (typeof body.error === "string" && body.error.trim()) message = body.error.replace(/^conflict: /, "");
    } catch {
      // A non-JSON failure keeps the generic message.
    }
    throw new Error(message);
  }
  const probed = await getHarnessAccount(account.id, account.companyId);
  if (!probed) throw new Error("Harness account no longer exists");
  return probed;
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

