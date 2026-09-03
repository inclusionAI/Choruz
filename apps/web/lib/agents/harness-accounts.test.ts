import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";

import { describe, expect, it } from "vitest";

import {
  claudeUsageWindows,
  harnessAccountEnv,
  parseCodexProbeResults,
  probeCodex,
  type HarnessAccount,
} from "./harness-accounts";
import { displayUsageWindows } from "./harness-account-display";

const baseAccount: Pick<HarnessAccount, "id" | "driverType" | "profileKind"> = {
  id: "12345678-1234-1234-1234-123456789abc",
  driverType: "claude_terminal",
  profileKind: "isolated",
};

describe("harness account profiles", () => {
  it("maps isolated profiles to a single harness-specific env var", () => {
    const env = harnessAccountEnv(baseAccount);
    expect(Object.keys(env)).toEqual(["CLAUDE_CONFIG_DIR"]);
    expect(env.CLAUDE_CONFIG_DIR).toMatch(/12345678-1234-1234-1234-123456789abc\/claude$/);
    expect(harnessAccountEnv({ ...baseAccount, driverType: "codex_terminal" })).toEqual({
      CODEX_HOME: expect.stringMatching(/12345678-1234-1234-1234-123456789abc\/codex$/),
    });
    expect(harnessAccountEnv({ ...baseAccount, profileKind: "default" })).toEqual({});
  });

  it("normalizes exact Claude subscription windows without inventing values", () => {
    expect(claudeUsageWindows({
      five_hour: { utilization: 24, resets_at: "2026-09-02T01:00:00Z" },
      seven_day: { utilization: 61.5, resets_at: "2026-09-07T01:00:00Z" },
      model_scoped: [{ display_name: "Opus", utilization: 80, resets_at: null }],
      missing: { utilization: null, resets_at: null },
    })).toEqual([
      expect.objectContaining({ id: "five_hour", label: "5-hour", usedPercent: 24, remainingPercent: 76 }),
      expect.objectContaining({ id: "seven_day", label: "Weekly", usedPercent: 61.5, remainingPercent: 38.5 }),
      expect.objectContaining({ id: "model_scoped:Opus", label: "Opus", remainingPercent: 20 }),
    ]);
  });

  it("normalizes persisted Claude window labels from every snapshot producer", () => {
    expect(displayUsageWindows("claude_terminal", [
      { id: "nimbus_quill", label: "nimbus quill", usedPercent: 0, remainingPercent: 100, resetsAt: null, windowDurationMinutes: null },
      { id: "seven_day", label: "seven day", usedPercent: 39, remainingPercent: 61, resetsAt: null, windowDurationMinutes: null },
      { id: "five_hour", label: "five hour", usedPercent: 20, remainingPercent: 80, resetsAt: null, windowDurationMinutes: null },
    ])).toEqual([
      expect.objectContaining({ id: "five_hour", label: "5-hour" }),
      expect.objectContaining({ id: "seven_day", label: "Weekly" }),
      expect.objectContaining({ id: "nimbus_quill", label: "nimbus quill" }),
    ]);
  });

  it("puts the default Codex quota before named product windows", () => {
    const window = (id: string, label: string) => ({ id, label, usedPercent: 0, remainingPercent: 100, resetsAt: null, windowDurationMinutes: null });
    expect(displayUsageWindows("codex_terminal", [
      window("codex_bengalfox:primary", "GPT-5.3-Codex-Spark 5-hour"),
      window("codex_bengalfox:secondary", "GPT-5.3-Codex-Spark Weekly"),
      window("codex:primary", "Weekly"),
    ]).map(({ id }) => id)).toEqual([
      "codex:primary",
      "codex_bengalfox:primary",
      "codex_bengalfox:secondary",
    ]);
  });

  it("normalizes exact Codex account, models, and weekly window", () => {
    const result = parseCodexProbeResults(new Map([
      [1, { account: { type: "chatgpt", email: "work@example.com", planType: "team" } }],
      [2, { rateLimits: {}, rateLimitsByLimitId: {
        codex: { secondary: { usedPercent: 37, windowDurationMins: 10080, resetsAt: 1788336000 } },
      } }],
      [3, { data: [{ id: "gpt-5.6-codex", displayName: "GPT-5.6 Codex" }] }],
    ]));
    expect(result.identifier).toBe("work@example.com");
    expect(result.subscriptionType).toBe("team");
    expect(result.models).toEqual([{ id: "gpt-5.6-codex", label: "GPT-5.6 Codex" }]);
    expect(result.windows).toEqual([
      expect.objectContaining({ label: "Weekly", usedPercent: 37, remainingPercent: 63, windowDurationMinutes: 10080 }),
    ]);
  });

  it("keeps distinct Codex quota buckets identifiable and removes mirrored windows", () => {
    const result = parseCodexProbeResults(new Map([
      [1, { account: { type: "chatgpt", email: "work@example.com", planType: "pro" } }],
      [2, { rateLimits: {}, rateLimitsByLimitId: {
        codex: {
          limitId: "codex",
          primary: { usedPercent: 80, windowDurationMins: 10080, resetsAt: 1800000000 },
        },
        codex_bengalfox: {
          limitId: "codex_bengalfox",
          limitName: "GPT-5.3-Codex-Spark",
          primary: { usedPercent: 0, windowDurationMins: 300, resetsAt: 1800000300 },
          secondary: { usedPercent: 0, windowDurationMins: 10080, resetsAt: 1800000600 },
        },
        mirrored: {
          limitName: "Mirror",
          primary: { usedPercent: 80, windowDurationMins: 10080, resetsAt: 1800000000 },
        },
      } }],
      [3, { data: [{ id: "gpt-5.6-codex", displayName: "GPT-5.6 Codex" }] }],
    ]));

    expect(result.windows).toEqual([
      expect.objectContaining({ id: "codex:primary", label: "Weekly", remainingPercent: 20 }),
      expect.objectContaining({ id: "codex_bengalfox:primary", label: "GPT-5.3-Codex-Spark 5-hour", remainingPercent: 100 }),
      expect.objectContaining({ id: "codex_bengalfox:secondary", label: "GPT-5.3-Codex-Spark Weekly", remainingPercent: 100 }),
    ]);
  });

  it("fails closed when Codex supplies no exact quota windows", () => {
    expect(() => parseCodexProbeResults(new Map([
      [1, { account: { type: "chatgpt", email: "work@example.com" } }],
      [2, { rateLimits: {} }],
      [3, { data: [] }],
    ]))).toThrow("no exact rate-limit windows");
  });

  it("uses JSON-RPC envelopes for the real Codex app-server transport", async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), "choruz-codex-account-probe-"));
    const binary = path.join(root, "codex");
    const fixture = `#!/usr/bin/env node
let buffer = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  buffer += chunk;
  const lines = buffer.split(/\\r?\\n/);
  buffer = lines.pop() || "";
  for (const line of lines) {
    if (!line.trim()) continue;
    const message = JSON.parse(line);
    if (message.jsonrpc !== "2.0") process.exit(20);
    const responses = {
      "initialize": { userAgent: "fixture" },
      "account/read": { account: { type: "chatgpt", email: "fixture@example.com", planType: "team" } },
      "account/rateLimits/read": { rateLimits: { primary: { usedPercent: 25, windowDurationMins: 10080, resetsAt: 1788336000 } } },
      "model/list": { data: [{ id: "fixture-model", displayName: "Fixture Model" }] }
    };
    if (message.id !== undefined && responses[message.method]) {
      process.stdout.write(JSON.stringify({ id: message.id, result: responses[message.method] }) + "\\n");
    }
  }
});
`;
    try {
      await fs.writeFile(binary, fixture, { mode: 0o755 });
      const result = await probeCodex(binary, { ...process.env } as Record<string, string>);
      expect(result).toMatchObject({
        identifier: "fixture@example.com",
        subscriptionType: "team",
        models: [{ id: "fixture-model", label: "Fixture Model" }],
        windows: [expect.objectContaining({ label: "Weekly", remainingPercent: 75 })],
      });
    } finally {
      await fs.rm(root, { recursive: true, force: true });
    }
  });
});
