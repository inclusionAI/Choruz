import type { Conversation, Principal } from "./choruz-types";

/** Display name for a principal id: the viewer, a known agent, or a short id. */
export function principalName(
  principal: Principal,
  agents: Principal[],
  id: string,
): string {
  if (principal.id === id) return principal.name;
  const agent = agents.find((a) => a.id === id);
  if (agent) return agent.name;
  return id.slice(0, 8);
}

/** A conversation's explicit name, else the other members' names. */
export function conversationDisplayName(
  conv: Conversation,
  principal: Principal,
  agents: Principal[],
): string {
  if (conv.name?.trim()) return conv.name;
  const memberIds = Object.keys(conv.members).filter((id) => id !== principal.id);
  if (memberIds.length === 0) return principal.name;
  return memberIds.map((id) => principalName(principal, agents, id)).join(", ");
}

export function isAgent(agents: Principal[], id: string): boolean {
  return agents.some((a) => a.id === id);
}

/** The other member of a direct conversation, or undefined for solo chats. */
export function directPeerId(conv: Conversation, principalId: string): string | undefined {
  return Object.keys(conv.members).find((id) => id !== principalId);
}
