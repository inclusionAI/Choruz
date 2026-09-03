import type { Conversation, Principal, RuntimeBindingInfo } from "../api/choruz-types";
import { directPeerId, isAgent } from "../api/principals";

/**
 * Bindings whose conversation is the raw terminal transcript, not messages.
 * The gateway fills `interaction_mode` for every binding from the stored
 * value or the driver it serves over a PTY, so plugin drivers need no list
 * here.
 */
export function bindingUsesTerminalTranscript(
  binding: { interaction_mode?: "message" | "terminal" | null },
): boolean {
  return binding.interaction_mode === "terminal";
}

/** The agent on the other side of a direct conversation, or null. */
export function agentPeerId(
  conversation: Conversation,
  principalId: string,
  agents: Principal[],
): string | null {
  if (conversation.conversation_type !== "direct") return null;
  const peerId = directPeerId(conversation, principalId);
  return peerId && isAgent(agents, peerId) ? peerId : null;
}

/**
 * Terminal-transcript binding that backs an agent DM: the binding bound to
 * this exact conversation, else the agent's direct binding, else any
 * terminal binding for that agent.
 */
export function findTerminalBinding(
  bindings: RuntimeBindingInfo[],
  conversationId: string,
  agentId: string,
): RuntimeBindingInfo | undefined {
  return preferredBinding(bindings.filter(bindingUsesTerminalTranscript), conversationId, agentId);
}

function preferredBinding(
  terminalBindings: RuntimeBindingInfo[],
  conversationId: string,
  agentId: string,
): RuntimeBindingInfo | undefined {
  return (
    terminalBindings.find((b) => b.conversation_id === conversationId) ??
    terminalBindings.find((b) => b.agent_principal_id === agentId && b.conversation_type === "direct") ??
    terminalBindings.find((b) => b.agent_principal_id === agentId)
  );
}

/**
 * Where a binding runs, as the UI names it: the paired runtime host's name,
 * "Remote machine" for a host this browser has not loaded, "This computer"
 * for a binding without a host.
 */
export function bindingMachineLabel(
  binding: { runtime_host_id?: string | null } | undefined,
  hosts: readonly { id: string; name: string }[],
): string {
  if (!binding?.runtime_host_id) return "This computer";
  return hosts.find((host) => host.id === binding.runtime_host_id)?.name ?? "Remote machine";
}

/** Any binding that backs an agent DM, whichever transcript it shows. */
export function findAgentDmBinding(
  bindings: RuntimeBindingInfo[],
  conversationId: string,
  agentId: string,
): RuntimeBindingInfo | undefined {
  return preferredBinding(bindings, conversationId, agentId);
}

/**
 * Terminal bindings for a set of open conversations; the first occurrence
 * that has a binding wins. Every one stays mounted so switching tabs never
 * tears down its WebSocket and kills the backend PTY.
 */
export function openTerminalBindings(
  conversationIds: Iterable<string>,
  conversations: Conversation[],
  agents: Principal[],
  principalId: string,
  bindings: RuntimeBindingInfo[],
): { convId: string; bindingId: string }[] {
  const terminalBindings = bindings.filter(bindingUsesTerminalTranscript);
  const conversationById = new Map(conversations.map((c) => [c.id, c]));
  const result: { convId: string; bindingId: string }[] = [];
  const seen = new Set<string>();
  for (const convId of conversationIds) {
    if (seen.has(convId)) continue;
    const conversation = conversationById.get(convId);
    if (!conversation) continue;
    const agentId = agentPeerId(conversation, principalId, agents);
    if (!agentId) continue;
    const binding = preferredBinding(terminalBindings, conversation.id, agentId);
    if (binding) {
      result.push({ convId: conversation.id, bindingId: binding.id });
      seen.add(convId);
    }
  }
  return result;
}
