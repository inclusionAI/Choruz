import { describe, expect, it } from "vitest";

import type { Conversation, Principal } from "../api/choruz-types";
import { mentionedAgentIds } from "./mentions";

const agent = (id: string, name: string): Principal =>
  ({ id, name, principal_type: "agent" }) as unknown as Principal;
const human = (id: string, name: string): Principal =>
  ({ id, name, principal_type: "human" }) as unknown as Principal;

const agents = [agent("ada", "Ada"), agent("bob", "Bob")];
const group = {
  conversation_type: "group",
  members: { ada: {}, pat: {}, bob: {} },
} as unknown as Conversation;

describe("mentionedAgentIds", () => {
  it("collects every agent whose @name appears, case-insensitively", () => {
    expect([...mentionedAgentIds(group, agents, "hey @ada and @BOB")]).toEqual(["ada", "bob"]);
  });

  it("expands @all to every agent member of the group, skipping humans", () => {
    expect([...mentionedAgentIds(group, agents, "@all standup")].sort()).toEqual(["ada", "bob"]);
  });

  it("ignores agents that are not members on @all", () => {
    const outsider = [...agents, agent("zed", "Zed")];
    expect([...mentionedAgentIds(group, outsider, "@all")].sort()).toEqual(["ada", "bob"]);
  });

  it("mentions nobody in a direct conversation or without a conversation", () => {
    const direct = { conversation_type: "direct", members: { ada: {} } } as unknown as Conversation;
    expect(mentionedAgentIds(direct, agents, "@ada hi").size).toBe(0);
    expect(mentionedAgentIds(null, agents, "@ada hi").size).toBe(0);
  });
});
