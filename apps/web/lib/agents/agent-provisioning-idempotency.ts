import crypto from "node:crypto";

import type {
  AgentProvisioningJobContext,
  AgentProvisioningStepRecord,
  AgentProvisioningStepRecorder,
  ProvisionRequestBody,
} from "./agent-provisioning";
import { postgresQueryClient } from "../groups/group-provisioning-db";
import type { JsonValue } from "../groups/group-provisioning-store";
import type { QueryClient } from "../groups/group-provisioning-store";

type CheckpointRow = {
  request_fingerprint: string;
  step_results_json: Record<string, AgentProvisioningStepRecord> | null;
};

type LockConnection = QueryClient & { release: () => void };
type LockPool = QueryClient & { connect: () => Promise<LockConnection> };

export class ProvisioningIdempotencyConflictError extends Error {
  constructor() {
    super("This idempotency key was already used for a different provisioning request.");
    this.name = "ProvisioningIdempotencyConflictError";
  }
}

export async function withProvisioningIdempotency<T>(
  actorId: string,
  body: ProvisionRequestBody,
  action: (context?: {
    jobContext: AgentProvisioningJobContext;
    stepRecorder: AgentProvisioningStepRecorder;
  }) => Promise<T>,
): Promise<T> {
  const idempotencyKey = body.idempotency_key;
  if (!idempotencyKey) return action();
  const fingerprint = requestFingerprint(body);
  return runDurableRequest(actorId, idempotencyKey, fingerprint, action);
}

async function runDurableRequest<T>(
  actorId: string,
  idempotencyKey: string,
  requestFingerprint: string,
  action: (context: {
    jobContext: AgentProvisioningJobContext;
    stepRecorder: AgentProvisioningStepRecorder;
  }) => Promise<T>,
): Promise<T> {
  const pool = await postgresQueryClient() as LockPool;
  const connection = await pool.connect();
  const lockScope = `agent-provisioning:${actorId}:${idempotencyKey}`;
  await connection.query("SELECT pg_advisory_lock(hashtextextended($1, 0))", [lockScope]);
  try {
    await connection.query(
      `INSERT INTO agent_provisioning_checkpoint
         (actor_id, idempotency_key, request_fingerprint)
       VALUES ($1, $2, $3)
       ON CONFLICT (actor_id, idempotency_key) DO NOTHING`,
      [actorId, idempotencyKey, requestFingerprint],
    );
    const row = await readCheckpoint(connection, actorId, idempotencyKey);
    if (!row || row.request_fingerprint !== requestFingerprint) {
      throw new ProvisioningIdempotencyConflictError();
    }

    const jobId = crypto.createHash("sha256").update(`${actorId}:${idempotencyKey}`).digest("hex");
    return await action({
      jobContext: { jobId, roleSlotId: "request", idempotencyKey },
      stepRecorder: {
        async readStep<TStep extends JsonValue>(
          input: Omit<AgentProvisioningStepRecord, "output">,
        ): Promise<TStep | null> {
          const latest = await readCheckpoint(connection, actorId, idempotencyKey);
          return (latest?.step_results_json?.[input.key]?.output as TStep | undefined) ?? null;
        },
        async recordStep<TStep extends JsonValue>(record: AgentProvisioningStepRecord<TStep>) {
          await connection.query(
            `UPDATE agent_provisioning_checkpoint
               SET step_results_json = step_results_json || jsonb_build_object($3::text, $4::jsonb),
                   updated_at = NOW()
             WHERE actor_id = $1 AND idempotency_key = $2`,
            [actorId, idempotencyKey, record.key, JSON.stringify(record)],
          );
        },
      },
    });
  } finally {
    try {
      await connection.query("SELECT pg_advisory_unlock(hashtextextended($1, 0))", [lockScope]);
    } finally {
      connection.release();
    }
  }
}

function requestFingerprint(body: ProvisionRequestBody): string {
  return crypto.createHash("sha256").update(stableJson(body)).digest("hex");
}

async function readCheckpoint(
  client: QueryClient,
  actorId: string,
  idempotencyKey: string,
): Promise<CheckpointRow | null> {
  const result = await client.query<CheckpointRow>(
    `SELECT request_fingerprint, step_results_json
       FROM agent_provisioning_checkpoint
      WHERE actor_id = $1 AND idempotency_key = $2`,
    [actorId, idempotencyKey],
  );
  return result.rows[0] ?? null;
}

function stableJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stableJson).join(",")}]`;
  if (value && typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>)
      .filter(([, entry]) => entry !== undefined)
      .sort(([left], [right]) => left.localeCompare(right));
    return `{${entries.map(([key, entry]) => `${JSON.stringify(key)}:${stableJson(entry)}`).join(",")}}`;
  }
  return JSON.stringify(value) ?? "null";
}
