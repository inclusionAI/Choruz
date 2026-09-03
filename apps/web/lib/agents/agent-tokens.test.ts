import { mkdtemp, readFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { persistAgentToken } from "./agent-tokens";

describe("persistAgentToken", () => {
  const originalRuntimeDir = process.env.CHORUZ_RUNTIME_DIR;
  const runtimeDirs: string[] = [];

  afterEach(async () => {
    if (originalRuntimeDir === undefined) {
      delete process.env.CHORUZ_RUNTIME_DIR;
    } else {
      process.env.CHORUZ_RUNTIME_DIR = originalRuntimeDir;
    }

    await Promise.all(
      runtimeDirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })),
    );
  });

  it("preserves every entry written by concurrent provisioning calls", async () => {
    const runtimeDir = await mkdtemp(path.join(tmpdir(), "echat-agent-tokens-"));
    runtimeDirs.push(runtimeDir);
    process.env.CHORUZ_RUNTIME_DIR = runtimeDir;

    const entries = Array.from({ length: 64 }, (_, index) => ({
      id: `agent-${index}`,
      secret: `agt_secret_${index}`,
    }));

    await Promise.all(
      entries.map(({ id, secret }) => persistAgentToken(id, secret)),
    );

    const tokensPath = path.join(runtimeDir, "agent_tokens.json");
    const raw = await readFile(tokensPath, "utf-8");
    const tokens = JSON.parse(raw) as Record<string, string>;

    expect(tokens).toEqual(
      Object.fromEntries(entries.map(({ id, secret }) => [id, secret])),
    );
    expect((await stat(tokensPath)).mode & 0o777).toBe(0o600);
  });
});
