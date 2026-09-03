import { describe, expect, it } from "vitest";

import { applyPendingOverrides, reconcileFlags, withoutConversation, type FlagOverride } from "./conversation-flags";

type Pin = { conversation_id: string; pinned_at: string };
const sortKey = (pin: Pin) => pin.pinned_at;
const pin = (id: string, at: string): Pin => ({ conversation_id: id, pinned_at: at });

describe("applyPendingOverrides", () => {
  it("returns the server list untouched when nothing is pending", () => {
    const server = [pin("a", "2026-01-02T00:00:00Z")];
    expect(applyPendingOverrides(new Map(), server, Date.now(), sortKey)).toBe(server);
  });

  it("lets an unsettled override replace the server's entry and sorts newest first", () => {
    const overrides = new Map<string, FlagOverride<Pin>>([
      ["a", { entry: pin("a", "2026-01-03T00:00:00Z"), settledAt: null }],
    ]);
    const server = [pin("a", "2026-01-01T00:00:00Z"), pin("b", "2026-01-02T00:00:00Z")];
    expect(applyPendingOverrides(overrides, server, Date.now(), sortKey).map((p) => p.conversation_id)).toEqual(["a", "b"]);
    expect(overrides.has("a")).toBe(true);
  });

  it("removes the entry when the override cleared the flag", () => {
    const overrides = new Map<string, FlagOverride<Pin>>([["a", { entry: null, settledAt: null }]]);
    const server = [pin("a", "2026-01-01T00:00:00Z"), pin("b", "2026-01-02T00:00:00Z")];
    expect(applyPendingOverrides(overrides, server, Date.now(), sortKey).map((p) => p.conversation_id)).toEqual(["b"]);
  });

  it("keeps a settled override for snapshots that started before it settled", () => {
    const settledAt = 1_000;
    const overrides = new Map<string, FlagOverride<Pin>>([["a", { entry: null, settledAt }]]);
    const server = [pin("a", "2026-01-01T00:00:00Z")];
    expect(applyPendingOverrides(overrides, server, settledAt - 1, sortKey)).toEqual([]);
    expect(overrides.has("a")).toBe(true);
  });

  it("drops a settled override once a later snapshot already reflects it", () => {
    const settledAt = 1_000;
    const overrides = new Map<string, FlagOverride<Pin>>([["a", { entry: null, settledAt }]]);
    const server = [pin("a", "2026-01-01T00:00:00Z")];
    expect(applyPendingOverrides(overrides, server, settledAt + 1, sortKey)).toBe(server);
    expect(overrides.has("a")).toBe(false);
  });
});

describe("reconcileFlags", () => {
  const entry = (id: string) => ({ conversation_id: id });

  it("drops pins of archived or hidden conversations and archives of hidden ones", () => {
    const { pinned, archived } = reconcileFlags(
      [entry("kept"), entry("archived"), entry("hidden")],
      [entry("archived"), entry("hidden")],
      [entry("hidden")],
    );
    expect(pinned.map((p) => p.conversation_id)).toEqual(["kept"]);
    expect(archived.map((a) => a.conversation_id)).toEqual(["archived"]);
  });

  it("returns the same arrays when nothing has to change", () => {
    const pinned = [entry("a")];
    const archived = [entry("b")];
    const result = reconcileFlags(pinned, archived, [entry("c")]);
    expect(result.pinned).toBe(pinned);
    expect(result.archived).toBe(archived);
  });
});

describe("withoutConversation", () => {
  it("filters by conversation id", () => {
    expect(withoutConversation([pin("a", "x"), pin("b", "y")], "a").map((p) => p.conversation_id)).toEqual(["b"]);
  });
});
