import type { ComponentProps } from "react";

import { GitGraphSection } from "../../components/workspace/git-graph-section";
import type { ClientPlugin } from "../client-plugin";

export const workspaceGitPlugin = {
  id: "workspace-git",
  version: "1",
  requiredHostCapabilities: ["workspace-repository-access"],
  clientCapabilities: ["detail-tab", "git-graph"],
} as const satisfies ClientPlugin;

export const workspaceGitDetailTab = {
  id: "git",
  label: "Git",
} as const;

export function WorkspaceGitPanel(props: ComponentProps<typeof GitGraphSection>) {
  return <GitGraphSection {...props} />;
}
