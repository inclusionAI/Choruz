import { describe, expect, it } from "vitest";

import {
  buildSidebarConversationSections,
  conversationPreviewText,
  type ArchivedConversation,
  type HiddenConversation,
  type PinnedConversation,
} from "./sidebar-conversations";
import { conversationDisplayName } from "../api/principals";
import type {
  ChatMessage,
  Conversation,
  Principal,
  RuntimeBindingInfo,
} from "../api/choruz-types";

const principal = makePrincipal("user-1", "Pat", "human");
const activeAgent = makePrincipal("agent-1", "Ada", "agent");
const disabledAgent = makePrincipal("agent-disabled", "Retired Bot", "agent", {
  disabled: true,
});
const secondAgent = makePrincipal("agent-2", "Turing", "agent");
const agents = [activeAgent, disabledAgent, secondAgent];

describe("conversationDisplayName", () => {
  it("matches the existing sidebar display-name behavior", () => {
    const named = conv("named", "group", [principal.id, activeAgent.id], {
      name: "Launch Room",
    });
    const direct = conv("direct", "direct", [principal.id, activeAgent.id]);
    const unknown = conv("unknown", "group", [principal.id, "human-abcdef"]);
    const selfOnly = conv("self", "direct", [principal.id]);

    expect(conversationDisplayName(named, principal, agents)).toBe("Launch Room");
    expect(conversationDisplayName(direct, principal, agents)).toBe("Ada");
    expect(conversationDisplayName(unknown, principal, agents)).toBe("human-ab");
    expect(conversationDisplayName(selfOnly, principal, agents)).toBe("Pat");
  });
});

describe("buildSidebarConversationSections", () => {
  it("partitions pinned chats once and removes them from their original sections", () => {
    const direct = conv("direct", "direct", [principal.id, activeAgent.id]);
    const group = conv("group", "group", [principal.id, activeAgent.id], {
      name: "Product",
    });
    const pinned: PinnedConversation[] = [
      pin("direct", "2026-05-10T10:00:00Z"),
      pin("group", "2026-05-10T09:00:00Z"),
    ];

    const result = buildSidebarConversationSections({
      conversations: [direct, group],
      agents,
      principal,
      messagesByConv: {},
      pinnedConversations: pinned,
    });

    expect(result.pinned.conversationIds).toEqual(["direct", "group"]);
    expect(result.direct.conversationIds).toEqual([]);
    expect(result.group.conversationIds).toEqual([]);
    expect(result.allFilteredConversationIds).toEqual(["direct", "group"]);
    expect(result.conversationSectionById).toEqual({
      direct: "pinned",
      group: "pinned",
    });
  });

  it("sorts pinned chats by pinned_at descending", () => {
    const oldest = conv("oldest", "direct", [principal.id, activeAgent.id]);
    const newest = conv("newest", "group", [principal.id, secondAgent.id], {
      name: "Newest Group",
    });
    const middle = conv("middle", "direct", [principal.id, secondAgent.id]);

    const result = buildSidebarConversationSections({
      conversations: [oldest, newest, middle],
      agents,
      principal,
      messagesByConv: {
        oldest: [msg("oldest-msg", "oldest", "2026-05-20T12:00:00Z")],
      },
      pinnedConversations: [
        pin("oldest", "2026-05-01T09:00:00Z"),
        pin("newest", "2026-05-03T09:00:00Z"),
        pin("middle", "2026-05-02T09:00:00Z"),
      ],
    });

    expect(result.pinned.conversationIds).toEqual(["newest", "middle", "oldest"]);
  });

  it("moves archived chats into a recoverable section even when stale pin data exists", () => {
    const direct = conv("direct", "direct", [principal.id, activeAgent.id]);
    const group = conv("group", "group", [principal.id, secondAgent.id], {
      name: "Archived Group",
    });

    const result = buildSidebarConversationSections({
      conversations: [direct, group],
      agents,
      principal,
      messagesByConv: {},
      pinnedConversations: [pin("direct", "2026-05-10T09:00:00Z")],
      archivedConversations: [
        archive("direct", "2026-05-10T10:00:00Z"),
        archive("group", "2026-05-09T10:00:00Z"),
      ],
    });

    expect(result.archived.conversationIds).toEqual(["direct", "group"]);
    expect(result.pinned.conversationIds).toEqual([]);
    expect(result.direct.conversationIds).toEqual([]);
    expect(result.group.conversationIds).toEqual([]);
    expect(result.archived.defaultExpanded).toBe(false);
  });

  it("removes hidden Agent sessions from every normal sidebar section and search", () => {
    const direct = conv("direct", "direct", [principal.id, activeAgent.id]);
    const group = conv("group", "group", [principal.id, secondAgent.id], { name: "Visible Group" });
    const hidden: HiddenConversation[] = [{
      conversation_id: "direct",
      hidden_at: "2026-05-10T10:00:00Z",
    }];

    const result = buildSidebarConversationSections({
      conversations: [direct, group],
      agents,
      principal,
      messagesByConv: {},
      pinnedConversations: [pin("direct", "2026-05-10T09:00:00Z")],
      hiddenConversations: hidden,
      searchQuery: "ada",
    });

    expect(result.sections.map((section) => section.id)).not.toContain("hidden");
    expect(result.allFilteredConversationIds).toEqual([]);
    expect(result.conversationSectionById.direct).toBeUndefined();
  });

  it("sorts unpinned direct and group sections by latest loaded message time", () => {
    const staleDirect = conv("stale-direct", "direct", [principal.id, activeAgent.id], {
      created_at: "2026-05-01T00:00:00Z",
    });
    const freshDirect = conv("fresh-direct", "direct", [principal.id, secondAgent.id], {
      created_at: "2026-05-02T00:00:00Z",
    });
    const staleGroup = conv("stale-group", "group", [principal.id, activeAgent.id], {
      name: "Stale Group",
      created_at: "2026-05-01T00:00:00Z",
    });
    const freshGroup = conv("fresh-group", "group", [principal.id, secondAgent.id], {
      name: "Fresh Group",
      created_at: "2026-05-02T00:00:00Z",
    });

    const result = buildSidebarConversationSections({
      conversations: [staleDirect, freshDirect, staleGroup, freshGroup],
      agents,
      principal,
      messagesByConv: {
        "stale-direct": [msg("d1", "stale-direct", "2026-05-09T00:00:00Z")],
        "fresh-direct": [msg("d2", "fresh-direct", "2026-05-10T00:00:00Z")],
        "stale-group": [msg("g1", "stale-group", "2026-05-07T00:00:00Z")],
        "fresh-group": [msg("g2", "fresh-group", "2026-05-08T00:00:00Z")],
      },
    });

    expect(result.direct.conversationIds).toEqual(["fresh-direct", "stale-direct"]);
    expect(result.group.conversationIds).toEqual(["fresh-group", "stale-group"]);
  });

  it("falls back to created_at when no loaded messages exist", () => {
    const older = conv("older", "direct", [principal.id, activeAgent.id], {
      created_at: "2026-05-01T00:00:00Z",
    });
    const newer = conv("newer", "direct", [principal.id, secondAgent.id], {
      created_at: "2026-05-02T00:00:00Z",
    });

    const result = buildSidebarConversationSections({
      conversations: [older, newer],
      agents,
      principal,
      messagesByConv: {},
    });

    expect(result.direct.conversationIds).toEqual(["newer", "older"]);
  });

  it("keeps disabled-agent direct conversations hidden, including pinned ones", () => {
    const disabledDirect = conv("disabled-direct", "direct", [
      principal.id,
      disabledAgent.id,
    ]);
    const visibleDirect = conv("visible-direct", "direct", [
      principal.id,
      activeAgent.id,
    ]);

    const result = buildSidebarConversationSections({
      conversations: [disabledDirect, visibleDirect],
      agents,
      principal,
      messagesByConv: {},
      pinnedConversations: [pin("disabled-direct", "2026-05-03T00:00:00Z")],
    });

    expect(result.pinned.conversationIds).toEqual([]);
    expect(result.direct.conversationIds).toEqual(["visible-direct"]);
    expect(result.allFilteredConversationIds).toEqual(["visible-direct"]);
  });

  it("searches display names only and keeps matches in their sections", () => {
    const ada = conv("ada", "direct", [principal.id, activeAgent.id]);
    const previewOnlyMatch = conv("preview-only", "direct", [
      principal.id,
      secondAgent.id,
    ]);
    const group = conv("group", "group", [principal.id, secondAgent.id], {
      name: "Ada Planning",
    });

    const result = buildSidebarConversationSections({
      conversations: [ada, previewOnlyMatch, group],
      agents,
      principal,
      messagesByConv: {
        "preview-only": [
          msg("preview-match", "preview-only", "2026-05-03T00:00:00Z", "Ada in preview"),
        ],
      },
      searchQuery: "ada",
    });

    expect(result.direct.conversationIds).toEqual(["ada"]);
    expect(result.group.conversationIds).toEqual(["group"]);
    expect(result.allFilteredConversationIds).toEqual(["ada", "group"]);
    expect(result.direct.forceExpandedBySearch).toBe(true);
    expect(result.group.forceExpandedBySearch).toBe(true);
  });

  it("marks the active conversation section for initial expansion", () => {
    const direct = conv("direct", "direct", [principal.id, activeAgent.id]);
    const group = conv("group", "group", [principal.id, secondAgent.id], {
      name: "Active Group",
    });

    const result = buildSidebarConversationSections({
      conversations: [direct, group],
      agents,
      principal,
      messagesByConv: {},
      activeConvId: "group",
    });

    expect(result.activeSectionId).toBe("group");
    expect(result.group.forceExpandedByActive).toBe(true);
    expect(result.direct.forceExpandedByActive).toBe(false);
  });

  it("returns all filtered conversation ids for Manage Chats All across collapsed sections", () => {
    const pinnedDirect = conv("pinned-direct", "direct", [
      principal.id,
      activeAgent.id,
    ]);
    const direct = conv("direct", "direct", [principal.id, secondAgent.id]);
    const group = conv("group", "group", [principal.id, secondAgent.id], {
      name: "Group",
    });

    const result = buildSidebarConversationSections({
      conversations: [direct, group, pinnedDirect],
      agents,
      principal,
      messagesByConv: {},
      pinnedConversations: [pin("pinned-direct", "2026-05-03T00:00:00Z")],
    });

    expect(result.pinned.defaultExpanded).toBe(true);
    expect(result.direct.defaultExpanded).toBe(false);
    expect(result.group.defaultExpanded).toBe(false);
    expect(result.allFilteredConversationIds).toEqual([
      "pinned-direct",
      "direct",
      "group",
    ]);
  });

  it("hides empty Pinned Chats but keeps empty Direct Messages and Group Conversations renderable", () => {
    const result = buildSidebarConversationSections({
      conversations: [],
      agents,
      principal,
      messagesByConv: {},
    });

    expect(result.pinned.shouldRender).toBe(false);
    expect(result.direct.shouldRender).toBe(true);
    expect(result.group.shouldRender).toBe(true);
    expect(result.archived.shouldRender).toBe(false);
  });

  it("marks exact direct terminal conversations without treating groups or normal DMs as terminal DMs", () => {
    const terminalDirect = conv("terminal-direct", "direct", [
      principal.id,
      activeAgent.id,
    ]);
    const normalDirect = conv("normal-direct", "direct", [
      principal.id,
      secondAgent.id,
    ]);
    const groupWithTerminalAgent = conv("terminal-group", "group", [
      principal.id,
      activeAgent.id,
    ]);

    const result = buildSidebarConversationSections({
      conversations: [terminalDirect, normalDirect, groupWithTerminalAgent],
      agents,
      principal,
      messagesByConv: {},
      runtimeBindings: [
        binding("binding-terminal", "terminal-direct", activeAgent.id, "codex_terminal"),
        binding("binding-group", "terminal-group", activeAgent.id, "claude_terminal"),
      ],
    });

    expect(
      result.direct.conversations.find((item) => item.id === "terminal-direct")
        ?.isTerminalDirectMessage,
    ).toBe(true);
    expect(
      result.direct.conversations.find((item) => item.id === "normal-direct")
        ?.isTerminalDirectMessage,
    ).toBe(false);
    expect(
      result.group.conversations.find((item) => item.id === "terminal-group")
        ?.isTerminalDirectMessage,
    ).toBe(false);
  });
});

describe("conversationPreviewText", () => {
  it("suppresses only the empty terminal-DM placeholder and preserves real previews", () => {
    expect(
      conversationPreviewText({
        messages: [],
        principal,
        agents,
        hideEmptyPlaceholder: true,
      }),
    ).toBeNull();
    expect(conversationPreviewText({ messages: [], principal, agents })).toBe(
      "No messages yet",
    );
    expect(
      conversationPreviewText({
        messages: [
          msg(
            "transcript",
            "terminal-direct",
            "2026-05-03T00:00:00Z",
            "\u001b[32mhello from terminal\u001b[0m",
            "runtime_transcript",
          ),
        ],
        principal,
        agents,
        hideEmptyPlaceholder: true,
      }),
    ).toBe("hello from terminal");
  });
});

function makePrincipal(
  id: string,
  name: string,
  principal_type: Principal["principal_type"],
  overrides: Partial<Principal> = {},
): Principal {
  return {
    id,
    workspace_id: "workspace-1",
    principal_type,
    name,
    avatar_url: null,
    scopes: [],
    disabled: false,
    created_at: "2026-05-01T00:00:00Z",
    updated_at: "2026-05-01T00:00:00Z",
    ...overrides,
  };
}

function conv(
  id: string,
  conversation_type: Conversation["conversation_type"],
  memberIds: string[],
  overrides: Partial<Conversation> = {},
): Conversation {
  return {
    id,
    workspace_id: "workspace-1",
    conversation_type,
    name: null,
    description: null,
    avatar_url: null,
    creator_id: principal.id,
    created_at: "2026-05-01T00:00:00Z",
    updated_at: "2026-05-01T00:00:00Z",
    members: Object.fromEntries(
      memberIds.map((id) => [
        id,
        {
          principal_id: id,
          joined_at: "2026-05-01T00:00:00Z",
        },
      ]),
    ),
    ...overrides,
  };
}

function msg(
  id: string,
  conversation_id: string,
  created_at: string,
  content = id,
  content_type = "text",
): ChatMessage {
  return {
    id,
    workspace_id: "workspace-1",
    conversation_id,
    sender_id: principal.id,
    content,
    content_type,
    metadata: {},
    edited_at: null,
    edited_by: null,
    server_seq: 1,
    idempotency_key: id,
    created_at,
  };
}

function pin(conversation_id: string, pinned_at: string): PinnedConversation {
  return { conversation_id, pinned_at };
}

function archive(
  conversation_id: string,
  archived_at: string,
): ArchivedConversation {
  return { conversation_id, archived_at };
}

function binding(
  id: string,
  conversation_id: string,
  agent_principal_id: string,
  driver_type: string,
): RuntimeBindingInfo {
  return {
    id,
    workspace_id: "workspace-1",
    conversation_id,
    conversation_type: "direct",
    agent_principal_id,
    driver_type,
    // The gateway fills interaction_mode from the driver it serves over a PTY.
    interaction_mode: driver_type.endsWith("_terminal") ? "terminal" : "message",
    workspace_path: "/tmp/workspace",
    state: "running",
  };
}
