import type { Conversation, Principal } from "../api/choruz-types";
import { isAgent } from "../api/principals";

/**
 * Agents a group message addresses: every agent member on `@all`,
 * otherwise each agent whose `@name` appears. Direct conversations mention
 * nobody; the peer is addressed by the conversation itself. Mirrors the
 * backend router so the thinking indicator matches who actually wakes.
 */
export function mentionedAgentIds(
  conversation: Conversation | null | undefined,
  agents: Principal[],
  content: string,
): Set<string> {
  const ids = new Set<string>();
  if (conversation?.conversation_type !== "group") return ids;
  const lc = content.toLowerCase();
  if (lc.includes("@all")) {
    for (const memberId of Object.keys(conversation.members)) {
      if (isAgent(agents, memberId)) ids.add(memberId);
    }
  } else {
    for (const agent of agents) {
      if (lc.includes(`@${agent.name.toLowerCase()}`)) ids.add(agent.id);
    }
  }
  return ids;
}
