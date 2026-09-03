import { beforeEach, describe, expect, it, vi } from "vitest";

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

  it("uses Claude SDK model metadata without sending a prompt", async () => {
    const discoverClaude = vi.fn(async () => [
      {
        id: "sonnet",
        label: "Sonnet",
        resolvedModel: "claude-sonnet-5",
        capabilities: { effortLevels: ["low", "high"], adaptiveThinking: true },
      },
    ]);
    const runCommand = vi.fn();

    const result = await discoverDriverModels("claude_terminal", {
      env: { CHORUZ_CLAUDE_BINARY: "/opt/claude" },
      discoverClaude,
      runCommand,
    });

    expect(result).toMatchObject({
      status: "available",
      models: [{ id: "sonnet", resolvedModel: "claude-sonnet-5" }],
    });
    expect(discoverClaude).toHaveBeenCalledWith("/opt/claude");
    expect(runCommand).not.toHaveBeenCalled();
  });

  it("caches successful scans by driver", async () => {
    const discoverClaude = vi.fn(async () => [{ id: "opus", label: "Opus" }]);
    const options = {
      env: { CHORUZ_CLAUDE_BINARY: "claude" },
      discoverClaude,
    };

    await discoverDriverModels("claude_terminal", options);
    await discoverDriverModels("claude_terminal", options);

    expect(discoverClaude).toHaveBeenCalledTimes(1);
  });

  it("shares one in-flight scan between concurrent requests", async () => {
    let resolveModels: ((models: [{ id: string; label: string }]) => void) | undefined;
    const discoverClaude = vi.fn(() => new Promise<[{ id: string; label: string }]>(
      (resolve) => { resolveModels = resolve; },
    ));
    const options = {
      env: { CHORUZ_CLAUDE_BINARY: "claude" },
      discoverClaude,
    };

    const first = discoverDriverModels("claude_terminal", options);
    const second = discoverDriverModels("claude_terminal", options);
    resolveModels?.([{ id: "opus", label: "Opus" }]);

    await expect(Promise.all([first, second])).resolves.toEqual([
      expect.objectContaining({ status: "available" }),
      expect.objectContaining({ status: "available" }),
    ]);
    expect(discoverClaude).toHaveBeenCalledTimes(1);
  });

  it("classifies harness authentication failures without exposing raw errors", async () => {
    const result = await discoverDriverModels("claude_terminal", {
      env: { CHORUZ_CLAUDE_BINARY: "claude" },
      discoverClaude: async () => {
        throw new Error("OAuth session expired at /Users/alice/.claude");
      },
    });

    expect(result).toEqual({
      driverId: "claude_terminal",
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
    const startedAt = Date.now();
    await expect(runModelCommand(
      process.execPath,
      ["-e", "process.on('SIGTERM', () => {}); setInterval(() => {}, 1_000)"],
      50,
    )).rejects.toThrow("Model discovery timed out");
    expect(Date.now() - startedAt).toBeLessThan(1_000);
  });
});
