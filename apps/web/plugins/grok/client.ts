import type { ClientPlugin } from "../client-plugin";

export const grokPlugin = {
  id: "grok",
  version: "1",
  requiredHostCapabilities: ["grok-agent-driver"],
  clientCapabilities: ["agent-provisioning", "session-import"],
} as const satisfies ClientPlugin;
