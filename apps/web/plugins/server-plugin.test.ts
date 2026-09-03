import { describe, expect, it } from "vitest";

import { serverPluginEnabled } from "./server-plugin";

describe("serverPluginEnabled", () => {
  it("installs all built-ins on the core when the allowlist is unset", () => {
    expect(serverPluginEnabled("any-installed-plugin", undefined)).toBe(true);
  });

  it("uses an exact comma-separated allowlist", () => {
    expect(serverPluginEnabled("workspace-git", " workspace-git,remote-ssh ")).toBe(true);
    expect(serverPluginEnabled("agent-skills", "workspace-git,remote-ssh")).toBe(false);
    expect(serverPluginEnabled("remote-ssh", "")).toBe(false);
  });
});
