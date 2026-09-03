import { describe, expect, it } from "vitest";

import { visibleChannelTaskAssignees } from "./channel-task-assignees";
import type { Conversation, Principal } from "../api/choruz-types";

describe("visibleChannelTaskAssignees", () => {
  it("includes visible human channel members beyond the current user", () => {
    const currentUser = principal("human-1", "Pat", "human");
    const teammate = principal("human-2", "Robin", "human");
    const agent = principal("agent-1", "Ada", "agent");
    const internalAgent = principal("agent-internal", "Private Ada", "agent", {
      channel_visibility: "internal",
    });
    const disabledHuman = principal("human-disabled", "Disabled", "human", {
      disabled: true,
    });
    const otherHuman = principal("human-3", "Taylor", "human");
    const crossWorkspaceHuman = principal("human-cross-workspace", "Cross Workspace", "human", {
      workspace_id: "workspace-2",
    });
    const outsider = principal("human-outside", "Outsider", "human");

    const assignees = visibleChannelTaskAssignees({
      conversation: conv([
        currentUser.id,
        teammate.id,
        agent.id,
        internalAgent.id,
        disabledHuman.id,
        otherHuman.id,
        crossWorkspaceHuman.id,
      ]),
      principals: [
        currentUser,
        teammate,
        agent,
        internalAgent,
        disabledHuman,
        otherHuman,
        crossWorkspaceHuman,
        outsider,
      ],
    });

    expect(assignees.map((item) => item.id).sort()).toEqual([
      "agent-1",
      "human-1",
      "human-2",
      "human-3",
    ]);
  });
});

function principal(
  id: string,
  name: string,
  principalType: Principal["principal_type"],
  overrides: Partial<Principal> = {},
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
    ...overrides,
  };
}

function conv(memberIds: string[]): Conversation {
  return {
    id: "conv-1",
    workspace_id: "workspace-1",
    conversation_type: "group",
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
          joined_at: "2026-05-01T00:00:00Z",
        },
      ]),
    ),
  };
}
