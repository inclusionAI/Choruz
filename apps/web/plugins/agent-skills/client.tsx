import type { ComponentProps } from "react";

import { AgentSkillsList } from "../../components/agents/agent-skills-list";
import type { ClientPlugin } from "../client-plugin";

export const agentSkillsPlugin = {
  id: "agent-skills",
  version: "1",
  requiredHostCapabilities: ["agent-workspace-access"],
  clientCapabilities: ["agent-detail-tab", "agent-provisioning"],
} as const satisfies ClientPlugin;

export const agentSkillsDetailTab = {
  id: "skills",
  label: "Skills",
} as const;

export function AgentSkillsPanel(props: ComponentProps<typeof AgentSkillsList>) {
  return <AgentSkillsList {...props} />;
}
