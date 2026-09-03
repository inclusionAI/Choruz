import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ChatApp, optimisticChannelTask } from "./chat-app";
import type { ChannelTask, ChatMessage, ConsoleSnapshot, Conversation, Principal } from "../../lib/api/choruz-types";

const KANBAN_PLUGIN = {
  id: "kanban",
  version: "1",
  host_capabilities: ["channel-task-api", "channel-task-events"],
  client_capabilities: ["conversation-tab", "message-action"],
};

describe("ChatApp channel task tabs", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("does not render the conversation Tasks tab while the gate is disabled", () => {
    vi.spyOn(console, "debug").mockImplementation(() => {});
    const principal = makePrincipal("human-1", "Pat", "human");
    const agent = makePrincipal("agent-1", "Ada", "agent");
    const conversation = conv("group-1", "group", [principal.id, agent.id]);
    const html = renderToStaticMarkup(
      createElement(ChatApp, {
        initialSnapshot: {
          principal,
          agents: [agent],
          conversations: [conversation],
          messages_by_conversation: { [conversation.id]: [] },
          audit_logs: [],
          plugins: [],
        } satisfies ConsoleSnapshot,
        sessionToken: "test-token",
        runtimeBindings: [],
        initialActiveConversationId: conversation.id,
      }),
    );

    expect(html).toContain("Message Launch Room");
    expect(html).not.toContain("Conversation views");
    expect(html).not.toContain(">Tasks<");
  });

  it("renders the conversation Tasks tab when the host and client plugins match", () => {
    vi.spyOn(console, "debug").mockImplementation(() => {});
    const principal = makePrincipal("human-1", "Pat", "human");
    const agent = makePrincipal("agent-1", "Ada", "agent");
    const conversation = conv("group-1", "group", [principal.id, agent.id]);
    const html = renderToStaticMarkup(
      createElement(ChatApp, {
        initialSnapshot: {
          principal,
          agents: [agent],
          conversations: [conversation],
          messages_by_conversation: { [conversation.id]: [] },
          audit_logs: [],
          plugins: [KANBAN_PLUGIN],
        } satisfies ConsoleSnapshot,
        sessionToken: "test-token",
        runtimeBindings: [],
        initialActiveConversationId: conversation.id,
      }),
    );

    expect(html).toContain("Conversation views");
    expect(html).toContain(">Tasks<");
  });

  it("hides the terminal chat surface while the task view is selected", () => {
    vi.spyOn(console, "debug").mockImplementation(() => {});
    const principal = makePrincipal("human-1", "Pat", "human");
    const agent = makePrincipal("agent-1", "Ada", "agent");
    const conversation = conv("direct-1", "direct", [principal.id, agent.id]);
    const html = renderToStaticMarkup(
      createElement(ChatApp, {
        initialSnapshot: {
          principal,
          agents: [agent],
          conversations: [conversation],
          messages_by_conversation: { [conversation.id]: [] },
          audit_logs: [],
          plugins: [KANBAN_PLUGIN],
        } satisfies ConsoleSnapshot,
        sessionToken: "test-token",
        runtimeBindings: [
          {
            id: "binding-1",
            conversation_id: conversation.id,
            conversation_type: "direct",
            agent_principal_id: agent.id,
            driver_type: "codex_terminal",
            interaction_mode: "terminal",
            workspace_path: "/tmp/workspace",
            state: "idle",
          },
        ],
        initialActiveConversationId: conversation.id,
        initialActiveConversationView: "tasks",
      }),
    );

    expect(html).toContain(">Tasks<");
    expect(html).toContain("No tasks yet.");
    expect(html).toContain('class="terminal-pane is-hidden"');
  });

  it("gives the terminal pane the whole chat column instead of an empty message panel", () => {
    vi.spyOn(console, "debug").mockImplementation(() => {});
    const principal = makePrincipal("human-1", "Pat", "human");
    const agent = makePrincipal("agent-1", "Ada", "agent");
    const conversation = conv("direct-1", "direct", [principal.id, agent.id]);
    const html = renderToStaticMarkup(
      createElement(ChatApp, {
        initialSnapshot: {
          principal,
          agents: [agent],
          conversations: [conversation],
          messages_by_conversation: { [conversation.id]: [] },
          audit_logs: [],
          plugins: [],
        } satisfies ConsoleSnapshot,
        sessionToken: "test-token",
        runtimeBindings: [
          {
            id: "binding-1",
            conversation_id: conversation.id,
            conversation_type: "direct",
            agent_principal_id: agent.id,
            driver_type: "codex_terminal",
            interaction_mode: "terminal",
            workspace_path: "/tmp/workspace",
            state: "idle",
          },
        ],
        initialActiveConversationId: conversation.id,
      }),
    );

    expect(html).toContain('<div class="terminal-pane">');
    expect(html).not.toContain("is-hidden");
    expect(html).not.toContain('id="conversation-chat-panel"');
    expect(html).not.toContain("Send to Ada terminal");
  });

  it("does not render the conversation Tasks tab for human-to-human direct conversations", () => {
    vi.spyOn(console, "debug").mockImplementation(() => {});
    const principal = makePrincipal("human-1", "Pat", "human");
    const teammate = makePrincipal("human-2", "Robin", "human");
    const conversation = conv("direct-1", "direct", [principal.id, teammate.id]);
    const html = renderToStaticMarkup(
      createElement(ChatApp, {
        initialSnapshot: {
          principal,
          agents: [],
          principals: [teammate],
          conversations: [conversation],
          messages_by_conversation: { [conversation.id]: [] },
          audit_logs: [],
          plugins: [KANBAN_PLUGIN],
        } satisfies ConsoleSnapshot,
        sessionToken: "test-token",
        runtimeBindings: [],
        initialActiveConversationId: conversation.id,
      }),
    );

    expect(html).not.toContain(">Tasks<");
  });

  it("renders create-from-message actions through the composed ChatApp path only for eligible conversations", () => {
    vi.spyOn(console, "debug").mockImplementation(() => {});
    const principal = makePrincipal("human-1", "Pat", "human");
    const agent = makePrincipal("agent-1", "Ada", "agent");
    const teammate = makePrincipal("human-2", "Robin", "human");
    const eligibleConversation = conv("group-1", "group", [principal.id, agent.id]);
    const humanDirectConversation = conv("direct-1", "direct", [principal.id, teammate.id]);

    const eligibleHtml = renderToStaticMarkup(
      createElement(ChatApp, {
        initialSnapshot: {
          principal,
          agents: [agent],
          principals: [teammate],
          conversations: [eligibleConversation],
          messages_by_conversation: {
            [eligibleConversation.id]: [msg({ conversation_id: eligibleConversation.id })],
          },
          audit_logs: [],
          plugins: [KANBAN_PLUGIN],
        } satisfies ConsoleSnapshot,
        sessionToken: "test-token",
        runtimeBindings: [],
        initialActiveConversationId: eligibleConversation.id,
        initialMessageActionsOpen: true,
      }),
    );
    const humanDirectHtml = renderToStaticMarkup(
      createElement(ChatApp, {
        initialSnapshot: {
          principal,
          agents: [],
          principals: [teammate],
          conversations: [humanDirectConversation],
          messages_by_conversation: {
            [humanDirectConversation.id]: [msg({ conversation_id: humanDirectConversation.id })],
          },
          audit_logs: [],
          plugins: [KANBAN_PLUGIN],
        } satisfies ConsoleSnapshot,
        sessionToken: "test-token",
        runtimeBindings: [],
        initialActiveConversationId: humanDirectConversation.id,
        initialMessageActionsOpen: true,
      }),
    );

    expect(eligibleHtml).toContain("Create task");
    expect(humanDirectHtml).toContain("Reply");
    expect(humanDirectHtml).not.toContain("Create task");
  });

  it("applies optimistic status and assignee updates before server reconciliation", () => {
    const agent = makePrincipal("agent-1", "Ada", "agent");
    const updated = optimisticChannelTask(
      task({
        status: "todo",
        assignee_principal_id: "human-1",
        assignee_name: "Pat",
        assignee_type: "human",
      }),
      { status: "blocked", assignee_principal_id: agent.id },
      [agent],
    );

    expect(updated).toMatchObject({
      status: "blocked",
      assignee_principal_id: "agent-1",
      assignee_name: "Ada",
      assignee_type: "agent",
    });
    expect(Date.parse(updated.updated_at)).toBeGreaterThanOrEqual(Date.parse("2026-05-01T00:00:00Z"));
  });
});

function makePrincipal(
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

function msg(overrides: Partial<ChatMessage> = {}): ChatMessage {
  return {
    id: "message-1",
    workspace_id: "workspace-1",
    conversation_id: "conv-1",
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
