import type { ComponentProps } from "react";

import { ChannelTaskBoard } from "../../components/channel-tasks/channel-task-board";
import { shouldShowChannelTasksTab } from "../../lib/channel-tasks/channel-tasks";
import type { ClientPlugin } from "../client-plugin";

export const kanbanPlugin = {
  id: "kanban",
  version: "1",
  requiredHostCapabilities: ["channel-task-api", "channel-task-events"],
  clientCapabilities: ["conversation-tab", "message-action"],
} as const satisfies ClientPlugin;

export const kanbanConversationTab = {
  id: "tasks" as const,
  label: "Tasks",
  isVisible: shouldShowChannelTasksTab,
};

export function KanbanBoard(props: ComponentProps<typeof ChannelTaskBoard>) {
  return <ChannelTaskBoard {...props} />;
}
