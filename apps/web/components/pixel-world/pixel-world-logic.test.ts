import { describe, expect, it } from "vitest";

import agentAtlas from "../../public/sprites/agents/agent_atlas.json";
import { hashStr } from "./pixel-sprites";
import { getAgentVisualDescriptor, isChoruzRosterAsset, MASTER_ASSETS } from "./agent-catalog";
import { generateColors, darken } from "./pixel-houses";
import { REMOTE_SERVER_INSTALL_COMMAND } from "../../lib/remote/remote-server-install";

describe("REMOTE_SERVER_INSTALL_COMMAND", () => {
  it("is a copy/paste executable command without explanatory comment text", () => {
    expect(REMOTE_SERVER_INSTALL_COMMAND).toBe("cargo build --release -p choruz-server && cp target/release/choruz-server ~/bin/");
    expect(REMOTE_SERVER_INSTALL_COMMAND).not.toContain("#");
  });
});

// ===========================================================================
// 1. hashStr — never produces negative modulo indexes
// ===========================================================================

describe("hashStr", () => {
  const inputs = [
    "",
    "a",
    "hello",
    "player",
    "00000000-0000-0000-0000-000000000000",
    "agent-123",
    "very-long-agent-id-with-many-characters-to-stress-the-hash",
    "\u00e9\u00e8\u00ea", // accented chars
    "AGENT_001",
    "claude_terminal",
  ];

  it("always returns a number", () => {
    for (const s of inputs) {
      expect(typeof hashStr(s)).toBe("number");
    }
  });

  it("never produces a negative index when used with array.length via Math.abs", () => {
    const arrayLengths = [1, 5, 10, 15, 17, 100, 256];
    for (const s of inputs) {
      const h = hashStr(s);
      for (const len of arrayLengths) {
        // The pattern used in pixel-houses.ts: Math.abs(hashStr(id)) % length
        const idx = Math.abs(h) % len;
        expect(idx).toBeGreaterThanOrEqual(0);
        expect(idx).toBeLessThan(len);
      }
    }
  });

  it("is deterministic — same input always gives same output", () => {
    for (const s of inputs) {
      expect(hashStr(s)).toBe(hashStr(s));
    }
  });
});

// ===========================================================================
// 2. getAgentVisualDescriptor — never returns undefined
// ===========================================================================

describe("getAgentVisualDescriptor", () => {
  const testIds = [
    "",
    "player",
    "claude_terminal",
    "codex_app_server",
    "acp",
    "claude_print",
    "codex_exec",
    "pi_terminal",
    "grok_terminal",
    "opencode_terminal",
    "00000000-0000-0000-0000-000000000000",
    "totally-unknown-agent",
    "AGENT_001",
    "AGENT_050",
    "AGENT_100",
    "x",
    "a very long agent identifier that should still hash correctly",
  ];

  it("never returns undefined for any string input", () => {
    for (const id of testIds) {
      const descriptor = getAgentVisualDescriptor(id);
      expect(descriptor).toBeDefined();
      expect(descriptor).not.toBeNull();
    }
  });

  it("always returns valid descriptor fields", () => {
    for (const id of testIds) {
      const d = getAgentVisualDescriptor(id);
      expect(typeof d.id).toBe("string");
      expect(typeof d.name).toBe("string");
      expect(typeof d.masterAsset).toBe("string");
      expect(typeof d.primaryColorHex).toBe("string");
      expect(d.masterAsset.length).toBeGreaterThan(0);
      expect(d.primaryColorHex).toMatch(/^#[0-9A-Fa-f]{6}$/);
    }
  });

  it("returns exact match for known system agents", () => {
    const d = getAgentVisualDescriptor("claude_terminal");
    expect(d.id).toBe("claude_terminal");
    expect(d.name).toBe("Terminal Host");
  });
});

describe("isChoruzRosterAsset", () => {
  it("accepts generated roster sheets but rejects the legacy atlas", () => {
    expect(isChoruzRosterAsset("/sprites/generated/agents/sheets/founder.png")).toBe(true);
    expect(isChoruzRosterAsset("/sprites/agents-atlas.png")).toBe(false);
    expect(isChoruzRosterAsset(undefined)).toBe(false);
  });
});

describe("legacy agent atlas", () => {
  it("contains every frame referenced by the agent catalog", () => {
    const atlasFrames = new Set(Object.keys(agentAtlas.frames));
    expect(Object.values(MASTER_ASSETS).every((frame) => atlasFrames.has(frame))).toBe(true);
  });
});

// ===========================================================================
// 3. generateColors — darken() never receives undefined
// ===========================================================================

describe("generateColors / darken", () => {
  const groupIds = [
    "",
    "proj-team",
    "general",
    "00000000-0000-0000-0000-000000000000",
    "x",
    "a-very-long-group-id-for-testing",
  ];

  it("darken returns a valid hex color for all palette colors", () => {
    // All colors that generateColors can pick from
    const allColors = [
      '#5E81AC', '#81A1C1', '#88C0D0', '#8FBCBB',
      '#A3BE8C', '#B48EAD', '#BF616A', '#D08770',
      '#EBCB8B', '#5B8C5A',
      '#3B4252', '#434C5E', '#4C566A', '#3F4A5C',
      '#4C566A', '#5A6578', '#3B4252', '#5E6B7E',
    ];
    for (const c of allColors) {
      const result = darken(c, 0.2);
      expect(result).toMatch(/^#[0-9a-f]{6}$/);
    }
  });

  it("generateColors never returns undefined fields", () => {
    for (const gid of groupIds) {
      const colors = generateColors(gid);
      expect(colors).toBeDefined();
      expect(colors.wall).toBeDefined();
      expect(colors.wallDark).toBeDefined();
      expect(colors.accent).toBeDefined();
      expect(colors.accentDark).toBeDefined();
      expect(colors.floor).toBeDefined();
      expect(colors.floorDark).toBeDefined();
      expect(colors.furniture).toBeDefined();
      expect(colors.furnitureDark).toBeDefined();

      // Verify all "dark" variants are valid hex
      expect(colors.accentDark).toMatch(/^#[0-9a-f]{6}$/);
      expect(colors.floorDark).toMatch(/^#[0-9a-f]{6}$/);
      expect(colors.furnitureDark).toMatch(/^#[0-9a-f]{6}$/);
    }
  });

  it("darken does not crash when called with any accent palette hex", () => {
    const amounts = [0, 0.1, 0.15, 0.2, 0.5, 1.0];
    const hexes = ['#5E81AC', '#2E3440', '#000000', '#FFFFFF'];
    for (const hex of hexes) {
      for (const amt of amounts) {
        expect(() => darken(hex, amt)).not.toThrow();
      }
    }
  });
});
