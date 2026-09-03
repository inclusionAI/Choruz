import { describe, expect, it } from "vitest";

import { thinkingMarkerClearIds } from "./thinking";

describe("thinkingMarkerClearIds", () => {
  it("clears a reply sender even when the client has not yet loaded that agent into its roster", () => {
    expect(thinkingMarkerClearIds([
      { sender_id: "imported-agent-not-in-roster" },
    ])).toEqual(new Set(["imported-agent-not-in-roster"]));
  });

  it("deduplicates senders from a recovered message page", () => {
    expect(thinkingMarkerClearIds([
      { sender_id: "agent-a" },
      { sender_id: "agent-b" },
      { sender_id: "agent-a" },
    ])).toEqual(new Set(["agent-a", "agent-b"]));
  });
});
