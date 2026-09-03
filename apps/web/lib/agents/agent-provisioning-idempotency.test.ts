import { readFileSync } from "node:fs";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentProvisioningStepRecord } from "./agent-provisioning";
import type { JsonValue, QueryClient, QueryStatement } from "../groups/group-provisioning-store";

const rows = new Map<string, {
  request_fingerprint: string;
  step_results_json: Record<string, AgentProvisioningStepRecord>;
}>();

const queryLog: string[] = [];
const lockTails = new Map<string, Promise<void>>();
const query = async <T extends object>(statement: QueryStatement, values: readonly unknown[] = []) => {
    const sql = typeof statement === "string" ? statement : statement.text;
    queryLog.push(sql);
    const key = `${values[0]}:${values[1]}`;
    if (sql.includes("INSERT INTO agent_provisioning_checkpoint")) {
      if (!rows.has(key)) {
        rows.set(key, {
          request_fingerprint: String(values[2]),
          step_results_json: {},
        });
      }
      return { rows: [] as T[] };
    }
    if (sql.includes("UPDATE agent_provisioning_checkpoint")) {
      const row = rows.get(key);
      if (row) row.step_results_json[String(values[2])] = JSON.parse(String(values[3]));
      return { rows: [] as T[] };
    }
    if (sql.includes("SELECT request_fingerprint")) {
      const row = rows.get(key);
      return { rows: (row ? [row] : []) as T[] };
    }
    throw new Error(`Unexpected query: ${sql}`);
};
const client: QueryClient & {
  connect: () => Promise<QueryClient & { release: () => void }>;
} = {
  query,
  async connect() {
    let unlockHeld: (() => void) | null = null;
    return {
      async query<T extends object>(statement: QueryStatement, values: readonly unknown[] = []) {
        const sql = typeof statement === "string" ? statement : statement.text;
        if (sql.includes("pg_advisory_lock")) {
          queryLog.push(sql);
          const scope = String(values[0]);
          const previous = lockTails.get(scope) ?? Promise.resolve();
          let unlock!: () => void;
          const held = new Promise<void>((resolve) => { unlock = resolve; });
          lockTails.set(scope, previous.then(() => held));
          await previous;
          unlockHeld = unlock;
          return { rows: [] as T[] };
        }
        if (sql.includes("pg_advisory_unlock")) {
          queryLog.push(sql);
          unlockHeld?.();
          unlockHeld = null;
          return { rows: [] as T[] };
        }
        return query<T>(statement, values);
      },
      release() {
        unlockHeld?.();
        unlockHeld = null;
      },
    };
  },
};

vi.mock("../groups/group-provisioning-db", () => ({
  postgresQueryClient: vi.fn(async () => client),
}));

import {
  ProvisioningIdempotencyConflictError,
  withProvisioningIdempotency,
} from "./agent-provisioning-idempotency";

const body = {
  name: "AI Manager",
  driver_type: "claude_terminal" as const,
  instructions: "Coordinate the company.",
  workspace_id: "company-1",
  idempotency_key: "company:company-1:ai-manager",
};

beforeEach(() => {
  rows.clear();
  queryLog.length = 0;
  lockTails.clear();
});

describe("withProvisioningIdempotency", () => {
  it("serializes concurrent requests and resumes their completed resource steps", async () => {
    let resourceCalls = 0;
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const action = vi.fn(async (context?: Parameters<Parameters<typeof withProvisioningIdempotency>[2]>[0]) => {
      if (!context) throw new Error("missing idempotency context");
      const input = {
        key: `${context.jobContext.jobId}:request:${context.jobContext.idempotencyKey}:create_principal`,
        step: "create_principal" as const,
        ...context.jobContext,
      };
      const existing = await context.stepRecorder.readStep<{ agentId: string }>(input);
      if (!existing) {
        resourceCalls += 1;
        await context.stepRecorder.recordStep({
          ...input,
          output: { agentId: "agent-1" } as JsonValue,
        });
        await gate;
      }
      return { agentId: "agent-1" };
    });

    const first = withProvisioningIdempotency("human-1", body, action);
    const second = withProvisioningIdempotency("human-1", body, action);
    release();

    await expect(Promise.all([first, second])).resolves.toEqual([
      { agentId: "agent-1" },
      { agentId: "agent-1" },
    ]);
    expect(action).toHaveBeenCalledTimes(2);
    expect(resourceCalls).toBe(1);
  });

  it("persists checkpoints so a later retry can resume completed steps", async () => {
    let firstAttempt = true;
    let createPrincipalCalls = 0;
    const action = async (context?: Parameters<Parameters<typeof withProvisioningIdempotency>[2]>[0]) => {
      if (!context) throw new Error("missing idempotency context");
      const input = {
        key: `${context.jobContext.jobId}:request:${context.jobContext.idempotencyKey}:create_principal`,
        step: "create_principal" as const,
        ...context.jobContext,
      };
      let principal = await context.stepRecorder.readStep<{ agentId: string }>(input);
      if (!principal) {
        createPrincipalCalls += 1;
        principal = { agentId: "agent-1" };
        await context.stepRecorder.recordStep({ ...input, output: principal as JsonValue });
      }
      if (firstAttempt) {
        firstAttempt = false;
        throw new Error("response connection lost");
      }
      return principal;
    };

    await expect(withProvisioningIdempotency("human-1", body, action)).rejects.toThrow("response connection lost");
    await expect(withProvisioningIdempotency("human-1", body, action)).resolves.toEqual({ agentId: "agent-1" });
    expect(createPrincipalCalls).toBe(1);
    expect(queryLog[0]).toContain("pg_advisory_lock");
    expect(queryLog.at(-1)).toContain("pg_advisory_unlock");
  });

  it("rejects reuse of a key with a different request body", async () => {
    await withProvisioningIdempotency("human-1", body, async () => "ok");
    await expect(withProvisioningIdempotency(
      "human-1",
      { ...body, driver_type: "codex_terminal" },
      async () => "unexpected",
    )).rejects.toBeInstanceOf(ProvisioningIdempotencyConflictError);
  });

  it("waits for the owner before rejecting a conflicting concurrent request", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const first = withProvisioningIdempotency("human-1", body, async () => {
      await gate;
      return "ok";
    });
    const conflicting = withProvisioningIdempotency(
      "human-1",
      { ...body, name: "Different Manager" },
      async () => "unexpected",
    );
    release();
    await expect(first).resolves.toBe("ok");
    await expect(conflicting).rejects.toBeInstanceOf(ProvisioningIdempotencyConflictError);
  });

  it("ships the durable checkpoint table in migrations", () => {
    const sql = readFileSync("../../migrations/V024__agent_provisioning_idempotency.sql", "utf8");
    expect(sql).toContain("CREATE TABLE IF NOT EXISTS agent_provisioning_checkpoint");
    expect(sql).toContain("PRIMARY KEY (actor_id, idempotency_key)");
  });
});
