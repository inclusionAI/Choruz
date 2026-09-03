import type { Conversation, Principal } from "../api/choruz-types";

export function shouldShowChannelTasksTab(input: {
  conversation: Conversation | null | undefined;
  principal: Principal;
  agents: Principal[];
  channelTasksEnabled: boolean;
}): boolean {
  const {
    conversation,
    principal,
    agents,
    channelTasksEnabled,
  } = input;

  if (!channelTasksEnabled || !conversation) {
    return false;
  }

  if (conversation.conversation_type === "group") {
    return true;
  }

  return Object.keys(conversation.members).some((memberId) => {
    if (memberId === principal.id) {
      return false;
    }
    const agent = agents.find((candidate) => candidate.id === memberId);
    return Boolean(
      agent &&
        !agent.disabled &&
        agent.channel_visibility !== "internal",
    );
  });
}
