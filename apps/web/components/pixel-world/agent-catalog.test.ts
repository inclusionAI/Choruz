import { expect, it } from "vitest";
import { CHORUZ_AGENT_SHEETS } from "./agent-catalog";

it("keeps a distinct sprite for each of the twenty roster identities", () => {
  const entries = Object.entries(CHORUZ_AGENT_SHEETS);
  expect(entries).toHaveLength(20);
  expect(new Set(entries.map(([, asset]) => asset)).size).toBe(entries.length);
  for (const [id, asset] of entries) {
    expect(asset).toBe(`/sprites/generated/agents/sheets/${id}.png`);
  }
});
