import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { MessageBubble } from "../../components/chat/message-bubble";
import {
  canCreateTaskFromMessage,
  createChannelTaskFromMessageRequest,
  createMessageTaskIdempotencyKey,
  defaultTaskTitleFromMessage,
  submitChannelTaskCreateFromMessage,
} from "./channel-task-create";
import type { ChannelTask, ChatMessage, Conversation, Principal } from "../api/choruz-types";

describe("channel task create-from-message helpers", () => {
  it("allows create actions only for eligible visible messages", () => {
    const conversation = conv("group", "group", ["human-1", "agent-1"]);
    const message = msg({ conversation_id: conversation.id, content_type: "text" });
    const assignees = [principal("agent-1", "Ada", "agent")];

    expect(canCreateTaskFromMessage({
      channelTasksVisible: true,
      conversation,
      message,
      visibleAssignees: assignees,
    })).toBe(true);
    expect(canCreateTaskFromMessage({
      channelTasksVisible: false,
      conversation,
      message,
      visibleAssignees: assignees,
    })).toBe(false);
    expect(canCreateTaskFromMessage({
      channelTasksVisible: true,
      conversation,
      message: msg({ conversation_id: conversation.id, content_type: "system" }),
      visibleAssignees: assignees,
    })).toBe(false);
    expect(canCreateTaskFromMessage({
      channelTasksVisible: true,
      conversation,
      message,
      visibleAssignees: [],
    })).toBe(false);
  });

  it("allows create actions for eligible direct conversations with an agent", () => {
    const conversation = conv("agent-direct", "direct", ["human-1", "agent-1"]);
    const message = msg({ conversation_id: conversation.id, content_type: "text" });

    expect(canCreateTaskFromMessage({
      channelTasksVisible: true,
      conversation,
      message,
      visibleAssignees: [principal("agent-1", "Ada", "agent")],
    })).toBe(true);
  });

  it("keeps human-to-human direct conversations hidden through the caller visibility flag", () => {
    const conversation = conv("human-direct", "direct", ["human-1", "human-2"]);
    expect(canCreateTaskFromMessage({
      channelTasksVisible: false,
      conversation,
      message: msg({ conversation_id: conversation.id }),
      visibleAssignees: [principal("human-2", "Robin", "human")],
    })).toBe(false);
  });

  it("prefills a bounded editable title from message content", () => {
    expect(defaultTaskTitleFromMessage("  > Follow up with Ada\n\n")).toBe("Follow up with Ada");
    expect(defaultTaskTitleFromMessage("")).toBe("Follow up on message");
    expect(defaultTaskTitleFromMessage("x".repeat(120))).toHaveLength(90);
  });

  it("generates message-scoped idempotency keys", () => {
    vi.stubGlobal("crypto", { randomUUID: () => "uuid-1" });
    expect(createMessageTaskIdempotencyKey("message-1")).toBe("message-task:message-1:uuid-1");
    vi.unstubAllGlobals();
  });

  it("sends the edited create-time title and required assignee in the create request", () => {
    const message = msg({ id: "message-9", content: "Original message title" });

    expect(createChannelTaskFromMessageRequest(
      { message, idempotencyKey: "message-task:message-9:uuid-1" },
      {
        title: "Edited launch handoff title",
        assigneePrincipalId: "agent-1",
        contextLabel: "Launch",
      },
    )).toEqual({
      message_id: "message-9",
      title: "Edited launch handoff title",
      assignee_principal_id: "agent-1",
      context_label: "Launch",
      idempotency_key: "message-task:message-9:uuid-1",
    });
  });

  it("surfaces create-from-message permission denial from the submit path", async () => {
    const message = msg({ id: "message-9", conversation_id: "agent-direct" });
    const createTask = vi.fn(async () => {
      throw new Error("permission denied");
    });

    await expect(submitChannelTaskCreateFromMessage({
      sessionToken: "test-token",
      draft: { message, idempotencyKey: "message-task:message-9:uuid-1" },
      payload: {
        title: "Edited launch handoff title",
        assigneePrincipalId: "agent-1",
        contextLabel: null,
      },
      createTask,
    })).resolves.toEqual({
      ok: false,
      error: "permission denied",
    });
    expect(createTask).toHaveBeenCalledWith("test-token", "agent-direct", {
      message_id: "message-9",
      title: "Edited launch handoff title",
      assignee_principal_id: "agent-1",
      context_label: null,
      idempotency_key: "message-task:message-9:uuid-1",
    });
  });

  it("returns the existing deduped task from the submit path without mutating the request title", async () => {
    const existingTask = task({
      task_id: "task-1",
      title: "Original task title",
      assignee_principal_id: "agent-1",
      assignee_name: "Ada",
    });
    const createTask = vi.fn(async () => existingTask);

    await expect(submitChannelTaskCreateFromMessage({
      sessionToken: "test-token",
      draft: {
        message: msg({ id: "message-9", conversation_id: "agent-direct" }),
        idempotencyKey: "message-task:message-9:uuid-1",
      },
      payload: {
        title: "Changed repeated attempt title",
        assigneePrincipalId: "agent-2",
        contextLabel: "Launch",
      },
      createTask,
    })).resolves.toEqual({
      ok: true,
      task: existingTask,
    });
  });

  it("renders the message action entry point only when create-from-message is available", () => {
    const currentUser = principal("human-1", "Pat", "human");
    const message = msg({ conversation_id: "group", content_type: "text" });
    const commonProps = {
      msg: message,
      idx: 0,
      allMsgs: [message],
      principal: currentUser,
      agents: [principal("agent-1", "Ada", "agent")],
      isTerminalChat: false,
      scrollToMessage: () => {},
      touchActiveId: null,
      onTouchStart: () => {},
      onTouchEnd: () => {},
      onTouchMove: () => {},
    };

    const visibleHtml = renderToStaticMarkup(
      createElement(MessageBubble, {
        ...commonProps,
        onCreateTaskFromMessage: () => {},
        initialActionsOpen: true,
      }),
    );
    const hiddenHtml = renderToStaticMarkup(
      createElement(MessageBubble, commonProps),
    );

    expect(visibleHtml).toContain('aria-label="Message actions"');
    expect(visibleHtml).toContain("Create task");
    expect(hiddenHtml).not.toContain('aria-label="Message actions"');
    expect(hiddenHtml).not.toContain("Create task");
  });
});

function principal(
  id: string,
  name: string,
  principalType: Principal["principal_type"],
): Principal {
  return {
    id,
    workspace_id: "workspace-1",
    principal_type: principalType,
    name,
    avatar_url: null,
    scopes: [],
    disabled: false,
    created_at: "2026-05-01T00:00:00Z",
    updated_at: "2026-05-01T00:00:00Z",
  };
}

function conv(
  id: string,
  conversationType: Conversation["conversation_type"],
  memberIds: string[],
): Conversation {
  return {
    id,
    workspace_id: "workspace-1",
    conversation_type: conversationType,
    name: "Launch Room",
    description: null,
    avatar_url: null,
    creator_id: memberIds[0],
    created_at: "2026-05-01T00:00:00Z",
    updated_at: "2026-05-01T00:00:00Z",
    members: Object.fromEntries(
      memberIds.map((memberId) => [
        memberId,
        {
          principal_id: memberId,
          role: memberId === memberIds[0] ? "owner" : "member",
          joined_at: "2026-05-01T00:00:00Z",
        },
      ]),
    ),
  };
}

function msg(overrides: Partial<ChatMessage> = {}): ChatMessage {
  return {
    id: "message-1",
    workspace_id: "workspace-1",
    conversation_id: "group",
    sender_id: "human-1",
    content: "Follow up with Ada",
    content_type: "text",
    metadata: {},
    edited_at: null,
    edited_by: null,
    server_seq: 1,
    idempotency_key: "message-1",
    created_at: "2026-05-01T00:00:00Z",
    ...overrides,
  };
}

function task(overrides: Partial<ChannelTask>): ChannelTask {
  return {
    task_id: "task-1",
    conversation_id: "conv-1",
    task_key: "TASK-1",
    title: "Follow up",
    status: "todo",
    assignee_principal_id: "human-1",
    assignee_type: "human",
    assignee_name: "Pat",
    source_kind: "message",
    source_message_id: "message-1",
    created_by: "human-1",
    created_by_type: "human",
    updated_by: "human-1",
    updated_by_type: "human",
    version: 1,
    created_at: "2026-05-01T00:00:00Z",
    updated_at: "2026-05-01T00:00:00Z",
    ...overrides,
  };
}
