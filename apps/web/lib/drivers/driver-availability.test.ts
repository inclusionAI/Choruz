import { describe, expect, it } from "vitest";
import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";

import {
  defaultPiRuntimeCheck,
  getDriverAvailability,
  resolveDriverBinary,
} from "./driver-availability";

describe("driver availability", () => {
  it("uses provisioning-compatible env/default binary resolution", () => {
    expect(resolveDriverBinary("claude_terminal", {})).toBe("claude");
    expect(resolveDriverBinary("codex_terminal", {})).toBe("codex");
    expect(resolveDriverBinary("codex_exec", {})).toBe("codex");
    expect(resolveDriverBinary("pi_terminal", {})).toBe("pi");
    expect(resolveDriverBinary("grok_terminal", {})).toBe("grok");
    expect(resolveDriverBinary("opencode_terminal", {})).toBe("opencode");
    expect(resolveDriverBinary("mathcode_terminal", {})).toBe("mathcode");
    expect(resolveDriverBinary("webhook_agent", {})).toBeUndefined();
    expect(resolveDriverBinary("codex_terminal", { CHORUZ_CODEX_BINARY: "/opt/bin/codex" })).toBe("/opt/bin/codex");
    expect(resolveDriverBinary("codex_terminal", {
      CHORUZ_CODEX_BINARY: "",
      CHORUZ_CODEX_CLI_PATH: "/runtime/bin/codex",
    })).toBe("/runtime/bin/codex");
    expect(resolveDriverBinary("claude_terminal", {
      CHORUZ_CLAUDE_CLI_PATH: "/runtime/bin/claude",
    })).toBe("/runtime/bin/claude");
  });

  it("reports user-facing status without requiring real CLIs in tests", async () => {
    const items = await getDriverAvailability({
      env: {
        CHORUZ_CLAUDE_BINARY: "/bin/claude",
        CHORUZ_CODEX_BINARY: "/bin/codex",
        CHORUZ_PI_BINARY: "/bin/pi",
        CHORUZ_GROK_BINARY: "/bin/grok",
        CHORUZ_OPENCODE_BINARY: "/bin/opencode",
        CHORUZ_MATHCODE_BINARY: "/bin/mathcode",
      },
      checkBinary: async (binary) => binary === "/bin/codex" || binary === "/bin/pi",
      checkPiRuntime: async () => ({ available: true }),
    });

    expect(items).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          label: "Claude",
          driverId: "claude_terminal",
          available: false,
          status: "unavailable",
          reason: "Claude CLI was not found.",
          setupHint: expect.stringContaining("CHORUZ_CLAUDE_BINARY"),
          envVar: "CHORUZ_CLAUDE_BINARY",
        }),
        expect.objectContaining({
          label: "Codex Terminal",
          driverId: "codex_terminal",
          available: true,
          status: "available",
          reason: "Codex Terminal CLI is available.",
          envVar: "CHORUZ_CODEX_BINARY",
        }),
        expect.objectContaining({
          label: "Pi Agent",
          driverId: "pi_terminal",
          available: true,
          envVar: "CHORUZ_PI_BINARY",
        }),
        expect.objectContaining({
          label: "Grok Build",
          driverId: "grok_terminal",
          available: false,
          envVar: "CHORUZ_GROK_BINARY",
        }),
        expect.objectContaining({
          label: "OpenCode",
          driverId: "opencode_terminal",
          available: false,
          envVar: "CHORUZ_OPENCODE_BINARY",
        }),
        expect.objectContaining({
          label: "MathCode",
          driverId: "mathcode_terminal",
          available: false,
          envVar: "CHORUZ_MATHCODE_BINARY",
        }),
        expect.objectContaining({
          label: "Webhook",
          driverId: "webhook_agent",
          available: true,
          reason: expect.stringContaining("do not require a local CLI"),
        }),
      ]),
    );
  });

  it("uses the supplied PATH for the default binary check", async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), "choruz-driver-path-"));
    const binary = path.join(root, "custom-codex");
    try {
      await fs.writeFile(binary, "#!/bin/sh\nexit 0\n");
      await fs.chmod(binary, 0o755);

      const items = await getDriverAvailability({
        env: {
          PATH: root,
          CHORUZ_CODEX_BINARY: "custom-codex",
        },
      });

      expect(items.find((item) => item.driverId === "codex_terminal"))
        .toMatchObject({ available: true, status: "available" });
    } finally {
      await fs.rm(root, { recursive: true, force: true });
    }
  });

  it("rejects the actual Node selected by Pi's env shebang when it misses the required runtime API", async () => {
    const fixture = await piNodeFixture({ version: "v22.14.0", zstd: false });
    try {
      const result = await defaultPiRuntimeCheck(fixture.pi, { PATH: fixture.path });
      expect(result.available).toBe(false);
      expect(result.reason).toContain("zlib.createZstdDecompress");
      expect(result.reason).toContain("v22.14.0");
      expect(result.setupHint).toContain("will not guess another global Node installation");
    } finally {
      await fs.rm(fixture.root, { recursive: true, force: true });
    }
  });

  it("accepts the actual Node selected by Pi when the engine and zstd API are present", async () => {
    const fixture = await piNodeFixture({ version: "v22.23.2", zstd: true });
    try {
      await expect(defaultPiRuntimeCheck(fixture.pi, { PATH: fixture.path })).resolves.toEqual({
        available: true,
      });
    } finally {
      await fs.rm(fixture.root, { recursive: true, force: true });
    }
  });
});

async function piNodeFixture(probe: { version: string; zstd: boolean }) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "choruz-pi-runtime-"));
  const bin = path.join(root, "bin");
  const dist = path.join(root, "dist");
  await fs.mkdir(bin);
  await fs.mkdir(dist);
  const pi = path.join(dist, "pi.js");
  await fs.writeFile(pi, "#!/usr/bin/env node\n");
  await fs.chmod(pi, 0o755);
  const node = path.join(bin, "node");
  await fs.writeFile(
    node,
    `#!/bin/sh\nprintf '%s' '${JSON.stringify(probe)}'\n`,
  );
  await fs.chmod(node, 0o755);
  return { root, pi, path: bin };
}
