import { existsSync, readFileSync } from "fs";
import path from "path";

import { type QueryClient } from "./group-provisioning-store";

type TransactionClient = QueryClient & { release(): void };
type QueryPool = QueryClient & { connect(): Promise<TransactionClient> };

let cachedPostgresClient: QueryPool | null = null;

export async function postgresQueryClient(): Promise<QueryPool> {
  if (cachedPostgresClient) return cachedPostgresClient;
  const pgModuleName = "pg";
  const pg = await import(pgModuleName) as {
    Pool: new (config: { connectionString: string }) => QueryPool;
  };
  cachedPostgresClient = new pg.Pool({ connectionString: postgresDatabaseUrl() });
  return cachedPostgresClient;
}

export async function withPostgresTransaction<T>(work: (client: QueryClient) => Promise<T>): Promise<T> {
  const pool = await postgresQueryClient();
  const client = await pool.connect();
  try {
    await client.query("BEGIN");
    const result = await work(client);
    await client.query("COMMIT");
    return result;
  } catch (error) {
    try {
      await client.query("ROLLBACK");
    } catch {
      // Preserve the original failure. The client is released below and pg will
      // discard it if the rollback left the connection unusable.
    }
    throw error;
  } finally {
    client.release();
  }
}

export function postgresDatabaseUrl(): string {
  const explicit = process.env.CHORUZ_DATABASE_URL?.trim();
  if (explicit) return explicit;

  const hostEnv = readHostEnv();
  const host = process.env.CHORUZ_PG_HOST?.trim() || hostEnv.CHORUZ_PG_HOST || "127.0.0.1";
  const port = process.env.CHORUZ_PG_PORT?.trim() || hostEnv.CHORUZ_PG_PORT || "5432";
  const db = process.env.CHORUZ_PG_DB?.trim() || hostEnv.CHORUZ_PG_DB || "choruz";
  const user = process.env.CHORUZ_PG_USER?.trim() || hostEnv.CHORUZ_PG_USER || process.env.USER || "choruz";
  return `postgres://${encodeURIComponent(user)}@${host}:${port}/${encodeURIComponent(db)}`;
}

function readHostEnv(): Record<string, string> {
  const envPath = findHostEnvPath(process.cwd());
  if (!envPath) return {};
  if (!existsSync(envPath)) return {};

  const values: Record<string, string> = {};
  for (const line of readFileSync(envPath, "utf8").split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const match = /^([A-Za-z_][A-Za-z0-9_]*)=(.*)$/.exec(trimmed);
    if (!match) continue;
    values[match[1]] = unquoteEnvValue(match[2].trim());
  }
  return values;
}

function findHostEnvPath(startDir: string): string | null {
  let dir = path.resolve(startDir);
  for (;;) {
    const candidate = path.join(dir, "infra", "host", ".env");
    if (existsSync(candidate)) return candidate;
    const parent = path.dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
}

function unquoteEnvValue(value: string): string {
  if (
    (value.startsWith('"') && value.endsWith('"')) ||
    (value.startsWith("'") && value.endsWith("'"))
  ) {
    return value.slice(1, -1);
  }
  return value;
}
