import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ChatInput } from "../chat/chat-input";
import {
  onlineConversation,
  onlineConversationId,
} from "./online-conversation";
import type { Principal } from "../../lib/api/choruz-types";

const principal: Principal = {
  id: "guest",
  workspace_id: "guest-workspace",
  name: "Guest",
  principal_type: "human",
  avatar_url: null,
  scopes: [],
  disabled: false,
  created_at: "2026-09-06T00:00:00Z",
  updated_at: "2026-09-06T00:00:00Z",
};
const conversation = onlineConversation(
  {
    id: "local-link",
    conversation_id: "owner-conversation",
    name: "Design group",
    role: "guest",
    status: "active",
  },
  principal,
);

describe("Online workspace presentation", () => {
  it("namespaces navigation and drafts without impersonating a local conversation owner", () => {
    expect(conversation.id).toBe(onlineConversationId("local-link"));
    expect(conversation.id).not.toBe("owner-conversation");
    expect(conversation.creator_id).toBe("");
    expect(conversation.members).toEqual({});
    expect(conversation.conversation_type).toBe("group");
  });

  it("uses the shared composer without offering unauthorized file transfer", () => {
    const props = {
      principal,
      agents: [],
      activeConv: conversation,
      placeholder: "Message Design group...",
      onSendMessage: async () => {},
      pendingFilesByConversation: {},
      setPendingFilesByConversation: () => {},
    };
    const online = renderToStaticMarkup(
      createElement(ChatInput, {
        ...props,
        attachmentsEnabled: false,
        disabled: true,
      }),
    );
    expect(online).toContain("chat-input-bar");
    expect(online).not.toContain('type="file"');
    expect(online).not.toContain("Attach file");
    expect(online).toMatch(/<textarea[^>]*disabled/);
    expect(online).toMatch(/<button[^>]*disabled[^>]*title="Send message"/);
    const local = renderToStaticMarkup(createElement(ChatInput, props));
    expect(local).toContain('type="file"');
    expect(local).toContain('aria-label="Attach file"');
    expect(local).not.toMatch(/<textarea[^>]*disabled/);
  });
});
