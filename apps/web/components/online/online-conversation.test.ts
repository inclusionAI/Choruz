import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ChatInput } from "../chat/chat-input";
import {
  onlineConversation,
  onlineConversationId,
  sameOnlineGroups,
  onlineRetryDelay,
} from "./online-conversation";
import { ApiRequestError } from "../../lib/api/choruz-api";
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
  it("retains equivalent group projections but notices visible fields and ordering", () => {
    const group = { id: "one", role: "guest" as const, status: "active" as const,
      name: "Design", conversation_id: "host", message_count: 1, latest_seq: 2,
      last_message: { content: "Ready", created_at: "2026-09-07T00:00:00Z" } };
    expect(sameOnlineGroups([], [])).toBe(true);
    expect(sameOnlineGroups([group], [structuredClone(group)])).toBe(true);
    for (const change of [{ id: "two" }, { role: "host" as const }, { status: "revoked" as const },
      { name: "Review" }, { conversation_id: "other" }, { message_count: 2 }, { latest_seq: 3 },
      { last_message: null }, { last_message: { ...group.last_message, content: "Changed" } },
      { last_message: { ...group.last_message, created_at: "2026-09-08T00:00:00Z" } }]) {
      expect(sameOnlineGroups([group], [{ ...group, ...change }])).toBe(false);
    }
    expect(sameOnlineGroups([group], [])).toBe(false);
    expect(sameOnlineGroups([group, { ...group, id: "two" }], [{ ...group, id: "two" }, group])).toBe(false);
  });

  it("backs off transient failures without retrying before Retry-After", () => {
    expect(onlineRetryDelay(new Error("offline"), 1)).toBe(6000);
    expect(onlineRetryDelay(new Error("offline"), 10)).toBe(60_000);
    expect(onlineRetryDelay(new ApiRequestError(429, "Wait", "90"), 1)).toBe(90_000);
    const now = Date.parse("2026-09-07T00:00:00Z");
    expect(onlineRetryDelay(new ApiRequestError(429, "Wait", "Mon, 07 Sep 2026 00:02:00 GMT"), 1, now)).toBe(120_000);
    expect(onlineRetryDelay(new ApiRequestError(503, "Unavailable", "invalid"), 1)).toBe(6000);
  });

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
