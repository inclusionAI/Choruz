import { beforeEach, describe, expect, it, vi } from "vitest";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as pause } from "node:timers/promises";

import {
  clearDriverModelDiscoveryCache,
  discoverDriverModels,
  parseCodexModelListResult,
  parseGrokModels,
  parseOpenCodeModels,
  parsePiModels,
  runModelCommand,
  validateModelId,
} from "./driver-models";

describe("driver model discovery", () => {
  beforeEach(() => clearDriverModelDiscoveryCache());

  it("uses the supplied runtime executable for model discovery", async () => {
    const discoverCodex = vi.fn(async () => [{ id: "model", label: "Model" }]);
    await discoverDriverModels("codex_terminal", {
      env: { CHORUZ_CODEX_BINARY: " ", CHORUZ_CODEX_CLI_PATH: " /runtime/codex " },
      discoverCodex,
    });
    expect(discoverCodex).toHaveBeenCalledWith("/runtime/codex");
  });

  it("preserves CLI model metadata without sending a prompt", async () => {
    const discoverCodex = vi.fn(async () => [
      {
        id: "test-model",
        label: "Test model",
        resolvedModel: "test-model-v1",
        capabilities: { effortLevels: ["low", "high"], adaptiveThinking: true },
      },
    ]);
    const runCommand = vi.fn();

    const result = await discoverDriverModels("codex_terminal", {
      env: { CHORUZ_CODEX_BINARY: "/opt/codex" },
      discoverCodex,
      runCommand,
    });

    expect(result).toMatchObject({
      status: "available",
      models: [{ id: "test-model", resolvedModel: "test-model-v1" }],
    });
    expect(discoverCodex).toHaveBeenCalledWith("/opt/codex");
    expect(runCommand).not.toHaveBeenCalled();
  });

  it("caches successful scans by driver", async () => {
    const discoverCodex = vi.fn(async () => [{ id: "test-model", label: "Test model" }]);
    const options = {
      env: { CHORUZ_CODEX_BINARY: "codex" },
      discoverCodex,
    };

    await discoverDriverModels("codex_terminal", options);
    await discoverDriverModels("codex_terminal", options);

    expect(discoverCodex).toHaveBeenCalledTimes(1);
  });

  it("shares one in-flight scan between concurrent requests", async () => {
    let resolveModels: ((models: [{ id: string; label: string }]) => void) | undefined;
    const discoverCodex = vi.fn(() => new Promise<[{ id: string; label: string }]>(
      (resolve) => { resolveModels = resolve; },
    ));
    const options = {
      env: { CHORUZ_CODEX_BINARY: "codex" },
      discoverCodex,
    };

    const first = discoverDriverModels("codex_terminal", options);
    const second = discoverDriverModels("codex_terminal", options);
    resolveModels?.([{ id: "test-model", label: "Test model" }]);

    await expect(Promise.all([first, second])).resolves.toEqual([
      expect.objectContaining({ status: "available" }),
      expect.objectContaining({ status: "available" }),
    ]);
    expect(discoverCodex).toHaveBeenCalledTimes(1);
  });

  it("classifies harness authentication failures without exposing raw errors", async () => {
    const result = await discoverDriverModels("codex_terminal", {
      env: { CHORUZ_CODEX_BINARY: "codex" },
      discoverCodex: async () => {
        throw new Error("OAuth session expired at /Users/alice/.codex");
      },
    });

    expect(result).toEqual({
      driverId: "codex_terminal",
      status: "auth_required",
      models: [],
      message: "Sign in to this harness before scanning its models.",
    });
    expect(JSON.stringify(result)).not.toContain("/Users/alice");
  });

  it("parses Pi's provider/model table", () => {
    expect(parsePiModels(`
provider    model                         context  max-out  thinking  images
anthropic   claude-sonnet-5               1M       128K     yes       yes
openai      gpt-5.6-codex                 400K     128K     yes       yes
`)).toEqual([
      { id: "anthropic/claude-sonnet-5", label: "claude-sonnet-5", provider: "anthropic" },
      { id: "openai/gpt-5.6-codex", label: "gpt-5.6-codex", provider: "openai" },
    ]);
  });

  it("parses Grok and OpenCode model output", () => {
    expect(parseGrokModels(`Available models:\n  * grok-4.6 (default)\n  - grok-4.5`)).toEqual([
      { id: "grok-4.6", label: "grok-4.6", description: "Default" },
      { id: "grok-4.5", label: "grok-4.5", description: undefined },
    ]);
    expect(parseOpenCodeModels("opencode/free\nopenrouter/anthropic/claude-sonnet\nnoise here\n")).toEqual([
      { id: "opencode/free", label: "free", provider: "opencode" },
      { id: "openrouter/anthropic/claude-sonnet", label: "anthropic/claude-sonnet", provider: "openrouter" },
    ]);
  });

  it("preserves Codex model IDs and advertised effort order", () => {
    expect(parseCodexModelListResult({
      data: [
        {
          id: "gpt-5.6-codex",
          displayName: "GPT-5.6 Codex",
          description: "Frontier coding model",
          supportedReasoningEfforts: [
            { reasoningEffort: "medium" },
            { reasoningEffort: "high" },
            { reasoningEffort: "xhigh" },
          ],
        },
      ],
      nextCursor: "page-2",
    })).toEqual({
      models: [
        {
          id: "gpt-5.6-codex",
          label: "GPT-5.6 Codex",
          description: "Frontier coding model",
          capabilities: { effortLevels: ["medium", "high", "xhigh"] },
        },
      ],
      nextCursor: "page-2",
    });
  });

  it("accepts exact provider model IDs while rejecting unsafe request shapes", () => {
    expect(validateModelId(undefined)).toBeNull();
    expect(validateModelId("claude-opus-5[1m]")).toBeNull();
    expect(validateModelId("openrouter/anthropic/claude-sonnet-5:fast")).toBeNull();
    expect(validateModelId(42)).toBe("Field `model` must be a string.");
    expect(validateModelId("bad\nmodel")).toBe("Field `model` cannot contain control characters.");
    expect(validateModelId("--sandbox")).toBe("Field `model` cannot start with `-`.");
    expect(validateModelId("x".repeat(257))).toBe("Field `model` must be 256 characters or fewer.");
  });

  it("enforces a hard timeout even when a harness ignores SIGTERM", async () => {
    const directory = await mkdtemp(join(tmpdir(), "choruz-model-timeout-"));
    const ready = join(directory, "ready");
    let pid: number | undefined;
    const alive = () => {
      try { process.kill(pid!, 0); return true; }
      catch (error) { if ((error as NodeJS.ErrnoException).code === "ESRCH") return false; throw error; }
    };
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const result = runModelCommand(process.execPath, ["-e",
        "process.on('SIGTERM', () => {}); require('fs').writeFileSync(process.argv[1], String(process.pid)); setInterval(() => {}, 1000)", ready,
      ], 50);
      const rejected = expect(result).rejects.toThrow("Model discovery timed out");
      for (let attempts = 0; pid === undefined; attempts++) {
        expect(attempts, "child must install its SIGTERM handler").toBeLessThan(200);
        const value = await readFile(ready, "utf8").catch(error => {
          if (error.code === "ENOENT") return "";
          throw error;
        });
        if (value) pid = Number(value);
        else await pause(10);
      }
      expect(alive()).toBe(true);
      await vi.advanceTimersByTimeAsync(50);
      await rejected;
      await vi.advanceTimersByTimeAsync(250);
      for (let attempts = 0; alive(); attempts++) {
        expect(attempts, "timed-out child must exit").toBeLessThan(200);
        await pause(10);
      }
    } finally {
      vi.useRealTimers();
      if (pid && alive()) {
        process.kill(pid, "SIGKILL");
        for (let attempts = 0; alive() && attempts < 200; attempts++) await pause(10);
      }
      await rm(directory, { recursive: true, force: true });
    }
  });
});
