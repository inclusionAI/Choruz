import type { ChatMessage, ChannelTask, Conversation, CreateChannelTaskFromMessageRequest, Principal } from "../api/choruz-types";

export type ChannelTaskCreateDraft = {
  message: ChatMessage;
  idempotencyKey: string;
};

export type ChannelTaskCreateFormPayload = {
  title: string;
  assigneePrincipalId: string;
  contextLabel: string | null;
};

export function canCreateTaskFromMessage(input: {
  channelTasksVisible: boolean;
  conversation: Conversation | null;
  message: ChatMessage;
  visibleAssignees: Principal[];
}): boolean {
  const { channelTasksVisible, conversation, message, visibleAssignees } = input;
  if (!channelTasksVisible || !conversation) return false;
  if (message.conversation_id !== conversation.id) return false;
  if (message.content_type === "system" || message.content_type === "runtime_transcript") return false;
  return visibleAssignees.length > 0;
}

export function defaultTaskTitleFromMessage(content: string): string {
  const compact = content
    .replace(/\s+/g, " ")
    .replace(/^[#>*\-\s]+/, "")
    .trim();
  if (!compact) return "Follow up on message";
  return compact.length > 90 ? `${compact.slice(0, 87).trimEnd()}...` : compact;
}

export function createMessageTaskIdempotencyKey(messageId: string): string {
  const random =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  return `message-task:${messageId}:${random}`;
}

export function createChannelTaskFromMessageRequest(
  draft: ChannelTaskCreateDraft,
  payload: ChannelTaskCreateFormPayload,
): CreateChannelTaskFromMessageRequest {
  return {
    message_id: draft.message.id,
    title: payload.title,
    assignee_principal_id: payload.assigneePrincipalId,
    context_label: payload.contextLabel,
    idempotency_key: draft.idempotencyKey,
  };
}

export async function submitChannelTaskCreateFromMessage(input: {
  sessionToken: string;
  draft: ChannelTaskCreateDraft;
  payload: ChannelTaskCreateFormPayload;
  createTask: (
    sessionToken: string,
    conversationId: string,
    request: CreateChannelTaskFromMessageRequest,
  ) => Promise<ChannelTask>;
}): Promise<
  | { ok: true; task: ChannelTask }
  | { ok: false; error: string }
> {
  try {
    const task = await input.createTask(
      input.sessionToken,
      input.draft.message.conversation_id,
      createChannelTaskFromMessageRequest(input.draft, input.payload),
    );
    return { ok: true, task };
  } catch (error) {
    return {
      ok: false,
      error: error instanceof Error ? error.message : "Unable to create task",
    };
  }
}
