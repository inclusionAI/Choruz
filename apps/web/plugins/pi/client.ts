import type { ClientPlugin } from "../client-plugin";

export const piPlugin = {
  id: "pi",
  version: "1",
  requiredHostCapabilities: ["pi-agent-driver"],
  clientCapabilities: ["agent-provisioning", "session-import"],
} as const satisfies ClientPlugin;
