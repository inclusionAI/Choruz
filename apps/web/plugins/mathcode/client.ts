import type { ClientPlugin } from "../client-plugin";

export const mathcodePlugin = {
  id: "mathcode",
  version: "1",
  requiredHostCapabilities: ["mathcode-agent-driver"],
  clientCapabilities: ["agent-provisioning"],
} as const satisfies ClientPlugin;
