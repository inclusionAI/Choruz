import { describe, expect, it } from "vitest";
import { COMPANY_SIGN_SIGNAL_COLORS, COMPANY_SIGN_SIGNAL_STROKES, darken, generateColors, generateHouse, type RoomColors } from "./pixel-houses";

describe("COMPANY_SIGN_SIGNAL_STROKES", () => {
  it("uses the three-stroke Signal Chorus motif within the 32×16 sign", () => {
    expect(COMPANY_SIGN_SIGNAL_STROKES).toHaveLength(3);
    for (const stroke of COMPANY_SIGN_SIGNAL_STROKES) {
      expect(stroke.every(([x, y]) => x >= 0 && x < 32 && y >= 0 && y < 16)).toBe(true);
    }
  });

  it("uses the approved navy, mint, and sky palette", () => {
    expect(COMPANY_SIGN_SIGNAL_COLORS).toEqual(["#102a6b", "#14b8a6", "#38bdf8"]);
  });
});

// generateHouse ---------------------------------------------------------------

describe("generateHouse — no room override", () => {
  it("returns the default 96×80 template when no room is provided", () => {
    const h = generateHouse("group-1", 5);
    expect(h.template.width).toBe(96);
    expect(h.template.height).toBe(80);
    expect(h.template.doorX).toBe(3);
    expect(h.template.doorY).toBe(4);
    expect(h.template.labelY).toBe(-4);
    expect(h.template.pixels).toEqual([]);
  });

  it("colours are deterministic per groupId regardless of memberCount", () => {
    const a = generateHouse("group-1", 3).colors;
    const b = generateHouse("group-1", 50).colors;
    expect(a).toEqual(b);
  });
});

describe("generateHouse — with room override", () => {
  it("derives template width / height from room.width × TILE_SIZE", () => {
    const room = {
      id: "r1",
      memberCount: 4,
      tileX: 0,
      tileY: 0,
      width: 6,
      height: 4,
      roomType: "workspace" as const,
      doorTileX: 3,
      doorTileY: 0,
      interactionPoints: [],
    };
    const h = generateHouse("g1", 4, room);
    // TILE_SIZE = 16
    expect(h.template.width).toBe(6 * 16);
    expect(h.template.height).toBe(4 * 16);
  });

  it("seeds an empty pixel grid sized pixelH × pixelW", () => {
    const room = {
      id: "r2",
      memberCount: 4,
      tileX: 0,
      tileY: 0,
      width: 4,
      height: 3,
      roomType: "lounge" as const,
      doorTileX: 0,
      doorTileY: 0,
      interactionPoints: [],
    };
    const h = generateHouse("g1", 4, room);
    expect(h.template.pixels.length).toBe(3 * 16);
    expect(h.template.pixels[0].length).toBe(4 * 16);
    // every pixel starts as 0
    expect(h.template.pixels.flat().every((p) => p === 0)).toBe(true);
  });

  it("converts room-relative door coordinates to tile coords", () => {
    // doorTileX = room.tileX + 2 → relative offset 2 → after divide-by-16
    // (since the function rounds down within the helper) the door lands at 2.
    const room = {
      id: "r3",
      memberCount: 4,
      tileX: 5,
      tileY: 7,
      width: 5,
      height: 5,
      roomType: "conference" as const,
      doorTileX: 7, // relative 2
      doorTileY: 7, // relative 0
      interactionPoints: [],
    };
    const h = generateHouse("g1", 4, room);
    expect(h.template.doorX).toBe(2);
    expect(h.template.doorY).toBe(0);
  });
});

// darken — extreme values ----------------------------------------------------

describe("darken — boundary cases", () => {
  it("clamps to 0 when amount >= 1", () => {
    expect(darken("#ffffff", 1)).toBe("#000000");
    expect(darken("#abcdef", 2)).toBe("#000000");
  });

  it("returns the same colour when amount is 0", () => {
    expect(darken("#1f4e8a", 0)).toBe("#1f4e8a");
  });

  it("never produces invalid hex (always 7 chars including #)", () => {
    for (const hex of ["#000000", "#ffffff", "#deadbe", "#012345"]) {
      for (const amt of [0, 0.1, 0.5, 0.9]) {
        const out = darken(hex, amt);
        expect(out).toMatch(/^#[0-9a-f]{6}$/);
      }
    }
  });
});

// generateColors — palette discipline ----------------------------------------

describe("generateColors", () => {
  it("returns a complete RoomColors object with all fields populated", () => {
    const c = generateColors("group-x");
    const required: Array<keyof RoomColors> = [
      "wall", "wallDark", "accent", "accentDark", "floor",
      "floorDark", "furniture", "furnitureDark", "screen",
      "screenGlow", "trim",
    ];
    for (const k of required) {
      expect(c).toHaveProperty(k);
      expect(c[k]).toMatch(/^#[0-9a-f]{6}$/i);
    }
  });

  it("is deterministic per groupId", () => {
    expect(generateColors("alpha")).toEqual(generateColors("alpha"));
  });

  it("accent and accentDark are different shades of the same hue", () => {
    const c = generateColors("g");
    expect(c.accent).not.toBe(c.accentDark);
  });
});
