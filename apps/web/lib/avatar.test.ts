import { describe, expect, it } from "vitest";
import { AVATAR_COLORS, avatarColor, avatarInitial, hashString } from "./avatar";

describe("hashString", () => {
  it("returns 0 for the empty string", () => {
    expect(hashString("")).toBe(0);
  });

  it("is deterministic for identical inputs", () => {
    expect(hashString("Alice")).toBe(hashString("Alice"));
  });

  it("differs across distinct names (sanity)", () => {
    const samples = ["a", "b", "c", "alice", "bob", "carol"];
    const hashes = samples.map(hashString);
    expect(new Set(hashes).size).toBeGreaterThan(1);
  });

  it("returns a non-negative integer (no NaN, no Infinity)", () => {
    const hashes = ["x", "long-name-here-with-symbols-!@#"].map(hashString);
    for (const h of hashes) {
      expect(Number.isInteger(h)).toBe(true);
      expect(h).toBeGreaterThanOrEqual(0);
    }
  });
});

describe("avatarColor", () => {
  it("always returns a hex color from the palette", () => {
    for (const name of ["Alice", "Bob", "carol-DAVE", "Élodie", "🎉"]) {
      expect(AVATAR_COLORS).toContain(avatarColor(name) as (typeof AVATAR_COLORS)[number]);
    }
  });

  it("is stable for the same name", () => {
    expect(avatarColor("Alice")).toBe(avatarColor("Alice"));
  });
});

describe("avatarInitial", () => {
  it("returns ? for empty / whitespace-only input", () => {
    expect(avatarInitial("")).toBe("?");
    expect(avatarInitial("   ")).toBe("?");
    expect(avatarInitial("\t\n")).toBe("?");
  });

  it("returns the first non-whitespace character upper-cased", () => {
    expect(avatarInitial("alice")).toBe("A");
    expect(avatarInitial("  bob")).toBe("B");
  });

  it("upper-cases non-ASCII Latin initials", () => {
    expect(avatarInitial("élodie")).toBe("É");
    // emoji surrogate pair: treating .toUpperCase() on the first code unit
    expect(avatarInitial("🎉")[0]).toBeDefined();
  });
});
