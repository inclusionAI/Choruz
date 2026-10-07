import { describe, expect, it } from "vitest";
import { workbenchConversations } from "./workbench-conversations";
import type { Conversation } from "../api/choruz-types";

describe("workbench navigation", () => {
  it("keeps groups out of the default projection without deleting them", () => {
    const conversations = [
      { id: "task", conversation_type: "direct" },
      { id: "team", conversation_type: "group" },
    ] as Conversation[];
    expect(workbenchConversations(conversations, false).map((item) => item.id)).toEqual(["task"]);
    expect(workbenchConversations(conversations, true)).toEqual(conversations);
    expect(conversations).toHaveLength(2);
  });
});
