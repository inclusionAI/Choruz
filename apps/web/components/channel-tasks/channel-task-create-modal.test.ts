import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ChannelTaskCreateModal } from "./channel-task-create-modal";
import type { ChatMessage, Principal } from "../../lib/api/choruz-types";

describe("ChannelTaskCreateModal", () => {
  it("prefills title, requires assignee, and exposes optional context", () => {
    const html = renderToStaticMarkup(
      createElement(ChannelTaskCreateModal, {
        message: msg({ content: "Please follow up with Ada about launch readiness." }),
        visibleAssignees: [principal("agent-1", "Ada", "agent")],
        submitting: false,
        error: null,
        onClose: () => {},
        onSubmit: () => {},
      }),
    );

    expect(html).toContain("Create task");
    expect(html).toContain('value="Please follow up with Ada about launch readiness."');
    expect(html).toContain("Select assignee");
    expect(html).toContain("Ada");
    expect(html).toContain("Context");
    expect(html).toContain("disabled");
  });

  it("keeps create disabled until a visible assignee is selected", () => {
    const html = renderToStaticMarkup(
      createElement(ChannelTaskCreateModal, {
        message: msg({ content: "Create a task from this" }),
        visibleAssignees: [],
        submitting: false,
        error: null,
        onClose: () => {},
        onSubmit: () => {},
      }),
    );

    expect(html).toContain("Select assignee");
    expect(html).toContain('<button class="btn-primary" type="button" disabled="">Create</button>');
  });

  it("renders permission errors without adding post-create editing controls", () => {
    const html = renderToStaticMarkup(
      createElement(ChannelTaskCreateModal, {
        message: msg(),
        visibleAssignees: [principal("human-1", "Pat", "human")],
        submitting: false,
        error: "permission denied",
        onClose: () => {},
        onSubmit: () => {},
      }),
    );

    expect(html).toContain("permission denied");
    expect(html).toContain('role="alert"');
    expect(html).not.toContain("Description");
    expect(html).not.toContain("Comment");
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
