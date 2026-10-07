import type { Conversation } from "../api/choruz-types";

/** Background collaboration is opt-in, including pinned and archived groups. */
export function workbenchConversations(conversations: Conversation[], includeCollaboration: boolean) {
  return includeCollaboration ? conversations : conversations.filter((conversation) => conversation.conversation_type === "direct");
}
