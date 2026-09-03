import { describe, expect, it } from "vitest";

import type { Conversation, Principal } from "./choruz-types";
import { conversationDisplayName, directPeerId, isAgent, principalName } from "./principals";

const viewer = makePrincipal("user-1", "Pat", "human");
const ada = makePrincipal("agent-1", "Ada", "agent");
const turing = makePrincipal("agent-2", "Turing", "agent");
const agents = [ada, turing];

describe("principalName", () => {
  it("resolves the viewer, known agents, and falls back to a short id", () => {
    expect(principalName(viewer, agents, "user-1")).toBe("Pat");
    expect(principalName(viewer, agents, "agent-2")).toBe("Turing");
    expect(principalName(viewer, agents, "0123456789abcdef")).toBe("01234567");
  });
});

describe("isAgent", () => {
  it("is true only for ids in the agent roster", () => {
    expect(isAgent(agents, "agent-1")).toBe(true);
    expect(isAgent(agents, "user-1")).toBe(false);
  });
});

describe("directPeerId", () => {
  it("returns the member that is not the viewer", () => {
    expect(directPeerId(conv("dm", "direct", ["user-1", "agent-1"]), "user-1")).toBe("agent-1");
  });

  it("is undefined for a conversation with only the viewer", () => {
    expect(directPeerId(conv("solo", "direct", ["user-1"]), "user-1")).toBeUndefined();
  });
});

describe("conversationDisplayName", () => {
  it("prefers an explicit name", () => {
    const named = conv("g", "group", ["user-1", "agent-1"], { name: "Launch Room" });
    expect(conversationDisplayName(named, viewer, agents)).toBe("Launch Room");
  });

  it("joins the other members' names when unnamed", () => {
    const group = conv("g", "group", ["user-1", "agent-1", "agent-2"]);
    expect(conversationDisplayName(group, viewer, agents)).toBe("Ada, Turing");
  });

  it("falls back to the viewer's name for a solo conversation", () => {
    expect(conversationDisplayName(conv("solo", "direct", ["user-1"]), viewer, agents)).toBe("Pat");
  });
});

function makePrincipal(id: string, name: string, principal_type: Principal["principal_type"]): Principal {
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
    creator_id: viewer.id,
    created_at: "2026-05-01T00:00:00Z",
    updated_at: "2026-05-01T00:00:00Z",
    members: Object.fromEntries(
      memberIds.map((memberId) => [memberId, { principal_id: memberId, joined_at: "2026-05-01T00:00:00Z" }]),
    ),
    ...overrides,
  };
}
