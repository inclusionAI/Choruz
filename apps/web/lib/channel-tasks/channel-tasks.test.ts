import { describe, expect, it } from "vitest";

import { shouldShowChannelTasksTab } from "./channel-tasks";
import type { Conversation, Principal } from "../api/choruz-types";

const principal = makePrincipal("human-1", "Pat", "human");
const visibleAgent = makePrincipal("agent-1", "Ada", "agent");
const internalAgent = makePrincipal("agent-internal", "Private Ada", "agent", {
  channel_visibility: "internal",
});
const disabledAgent = makePrincipal("agent-disabled", "Retired Ada", "agent", {
  disabled: true,
});
const human = makePrincipal("human-2", "Robin", "human");

describe("shouldShowChannelTasksTab", () => {
  it("hides the channel Tasks tab while the rollout gate is disabled", () => {
    const agents = [visibleAgent];

    for (const conversation of [
      conv("group", "group", [principal.id, visibleAgent.id]),
      conv("agent-direct", "direct", [principal.id, visibleAgent.id]),
      conv("human-direct", "direct", [principal.id, human.id]),
    ]) {
      expect(
        shouldShowChannelTasksTab({
          conversation,
          principal,
          agents,
          channelTasksEnabled: false,
        }),
      ).toBe(false);
    }
  });

  it("allows the tab only for eligible conversations after the gate is enabled", () => {
    const agents = [visibleAgent, internalAgent, disabledAgent];

    expect(
      shouldShowChannelTasksTab({
        conversation: conv("group", "group", [principal.id, human.id]),
        principal,
        agents,
        channelTasksEnabled: true,
      }),
    ).toBe(true);
    expect(
      shouldShowChannelTasksTab({
        conversation: conv("agent-direct", "direct", [
          principal.id,
          visibleAgent.id,
        ]),
        principal,
        agents,
        channelTasksEnabled: true,
      }),
    ).toBe(true);
    expect(
      shouldShowChannelTasksTab({
        conversation: conv("human-direct", "direct", [principal.id, human.id]),
        principal,
        agents,
        channelTasksEnabled: true,
      }),
    ).toBe(false);
    expect(
      shouldShowChannelTasksTab({
        conversation: conv("internal-agent-direct", "direct", [
          principal.id,
          internalAgent.id,
        ]),
        principal,
        agents,
        channelTasksEnabled: true,
      }),
    ).toBe(false);
    expect(
      shouldShowChannelTasksTab({
        conversation: conv("disabled-agent-direct", "direct", [
          principal.id,
          disabledAgent.id,
        ]),
        principal,
        agents,
        channelTasksEnabled: true,
      }),
    ).toBe(false);
  });
});

function makePrincipal(
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

function conv(
  id: string,
  conversationType: Conversation["conversation_type"],
  memberIds: string[],
): Conversation {
  return {
    id,
    workspace_id: "workspace-1",
    conversation_type: conversationType,
    name: null,
    description: null,
    avatar_url: null,
    creator_id: principal.id,
    created_at: "2026-05-01T00:00:00Z",
    updated_at: "2026-05-01T00:00:00Z",
    members: Object.fromEntries(
      memberIds.map((memberId) => [
        memberId,
        {
          principal_id: memberId,
          role: memberId === principal.id ? "owner" : "member",
          joined_at: "2026-05-01T00:00:00Z",
        },
      ]),
    ),
  };
}
