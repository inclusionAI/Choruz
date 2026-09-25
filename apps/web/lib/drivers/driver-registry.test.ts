import { describe, expect, it } from "vitest";
import { creatableAgentDriverIds, isTerminalDriver } from "./driver-registry";
import { resolveClientPluginIds } from "../../plugins/registry";

describe("optional harness plugins", () => {
  it("offers only core drivers without manifests, preserving existing terminal identities", () => {
    expect(creatableAgentDriverIds(new Set())).toEqual(["claude_terminal", "codex_terminal", "grok_terminal", "webhook_agent"]);
    expect(isTerminalDriver("pi_terminal")).toBe(true);
    expect(isTerminalDriver("opencode_terminal")).toBe(true);
  });

  it.each(["pi", "opencode", "mathcode"])("offers %s only with a compatible host manifest", (plugin) => {
    const manifest = { id: plugin, version: "1", host_capabilities: [`${plugin}-agent-driver`], client_capabilities: ["agent-provisioning", "session-import"] };
    expect(creatableAgentDriverIds(resolveClientPluginIds([manifest]))).toContain(`${plugin}_terminal`);
    expect(creatableAgentDriverIds(resolveClientPluginIds([{ ...manifest, version: "2" }]))).not.toContain(`${plugin}_terminal`);
  });
});
