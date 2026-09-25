import { describe, expect, it } from "vitest";

import { serverPluginEnabled } from "./server-plugin";

describe("serverPluginEnabled", () => {
  it("keeps optional harness plugins off when the allowlist is unset", () => {
    expect(serverPluginEnabled("mathcode", undefined)).toBe(true);
    expect(serverPluginEnabled("pi", undefined)).toBe(false);
    expect(serverPluginEnabled("opencode", undefined)).toBe(false);
  });

  it("uses an exact comma-separated allowlist", () => {
    expect(serverPluginEnabled("workspace-git", " workspace-git,remote-ssh ")).toBe(true);
    expect(serverPluginEnabled("agent-skills", "workspace-git,remote-ssh")).toBe(false);
    expect(serverPluginEnabled("remote-ssh", "")).toBe(false);
    expect(serverPluginEnabled("pi", "pi,opencode")).toBe(true);
    expect(serverPluginEnabled("opencode", "pi,opencode")).toBe(true);
  });
});
