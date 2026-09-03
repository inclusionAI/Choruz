import type { Conversation, Principal } from "../api/choruz-types";

export function visibleChannelTaskAssignees(input: {
  conversation: Conversation | null;
  principals: Principal[];
}): Principal[] {
  const { conversation, principals } = input;
  if (!conversation) return [];

  return dedupePrincipals(principals)
    .filter((candidate) => {
      const isMember = Boolean(conversation.members[candidate.id]);
      const isAssignableType = candidate.principal_type === "human" || candidate.principal_type === "agent";
      const isSameWorkspace = candidate.workspace_id === conversation.workspace_id;
      const isVisibleAgent =
        candidate.principal_type !== "agent" || candidate.channel_visibility !== "internal";
      return isMember && isAssignableType && isSameWorkspace && !candidate.disabled && isVisibleAgent;
    })
    .sort((left, right) => left.name.localeCompare(right.name));
}

export function dedupePrincipals(principals: Principal[]): Principal[] {
  const byId = new Map<string, Principal>();
  for (const principal of principals) {
    byId.set(principal.id, principal);
  }
  return [...byId.values()];
}
