import type { ClientPlugin } from "../client-plugin";

export const opencodePlugin = {
  id: "opencode",
  version: "1",
  requiredHostCapabilities: ["opencode-agent-driver"],
  clientCapabilities: ["agent-provisioning", "session-import"],
} as const satisfies ClientPlugin;
