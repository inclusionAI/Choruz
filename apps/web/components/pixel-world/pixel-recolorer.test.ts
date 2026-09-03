import { describe, expect, it } from "vitest";
import {
  hexToRgb,
  hslToRgb,
  rgbToHsl,
} from "./pixel-recolorer";

// hexToRgb --------------------------------------------------------------------

describe("hexToRgb", () => {
  it("parses lowercase 6-digit hex with leading #", () => {
    expect(hexToRgb("#ff0080")).toEqual({ r: 255, g: 0, b: 128 });
  });

  it("parses uppercase hex", () => {
    expect(hexToRgb("#FF0080")).toEqual({ r: 255, g: 0, b: 128 });
  });

  it("parses hex without leading #", () => {
    expect(hexToRgb("0a141e")).toEqual({ r: 10, g: 20, b: 30 });
  });

  it("returns black for invalid input", () => {
    expect(hexToRgb("not-a-color")).toEqual({ r: 0, g: 0, b: 0 });
    expect(hexToRgb("#fff")).toEqual({ r: 0, g: 0, b: 0 }); // 3-digit shorthand not supported
    expect(hexToRgb("")).toEqual({ r: 0, g: 0, b: 0 });
  });

  it("handles white and black extremes", () => {
    expect(hexToRgb("#000000")).toEqual({ r: 0, g: 0, b: 0 });
    expect(hexToRgb("#ffffff")).toEqual({ r: 255, g: 255, b: 255 });
  });
});

// rgbToHsl --------------------------------------------------------------------

describe("rgbToHsl", () => {
  it("returns h=s=0 for grey (achromatic)", () => {
    const [h, s, l] = rgbToHsl(128, 128, 128);
    expect(h).toBe(0);
    expect(s).toBe(0);
    expect(l).toBeCloseTo(128 / 255, 3);
  });

  it("pure red → h≈0, s=1, l=0.5", () => {
    const [h, s, l] = rgbToHsl(255, 0, 0);
    expect(h).toBeCloseTo(0, 3);
    expect(s).toBeCloseTo(1, 3);
    expect(l).toBeCloseTo(0.5, 3);
  });

  it("pure green → h≈1/3", () => {
    const [h] = rgbToHsl(0, 255, 0);
    expect(h).toBeCloseTo(1 / 3, 3);
  });

  it("pure blue → h≈2/3", () => {
    const [h] = rgbToHsl(0, 0, 255);
    expect(h).toBeCloseTo(2 / 3, 3);
  });

  it("white → l=1, s=0", () => {
    const [, s, l] = rgbToHsl(255, 255, 255);
    expect(s).toBe(0);
    expect(l).toBe(1);
  });
});

// hslToRgb --------------------------------------------------------------------

describe("hslToRgb", () => {
  it("returns equal RGB channels for s=0 (achromatic)", () => {
    const [r, g, b] = hslToRgb(0, 0, 0.5);
    expect(r).toBe(g);
    expect(g).toBe(b);
  });

  it("recovers pure red from h=0, s=1, l=0.5", () => {
    expect(hslToRgb(0, 1, 0.5)).toEqual([255, 0, 0]);
  });

  it("recovers pure green from h=1/3", () => {
    expect(hslToRgb(1 / 3, 1, 0.5)).toEqual([0, 255, 0]);
  });

  it("recovers pure blue from h=2/3", () => {
    expect(hslToRgb(2 / 3, 1, 0.5)).toEqual([0, 0, 255]);
  });
});

// round-trip ------------------------------------------------------------------

describe("rgbToHsl ↔ hslToRgb round-trip", () => {
  it("preserves saturated primaries within ±1 RGB unit", () => {
    for (const orig of [
      [255, 0, 0],
      [0, 255, 0],
      [0, 0, 255],
      [255, 200, 0],
      [10, 200, 100],
    ] as const) {
      const [h, s, l] = rgbToHsl(orig[0], orig[1], orig[2]);
      const [r, g, b] = hslToRgb(h, s, l);
      // Allow ±1 because rounding happens at hslToRgb's *255 step
      expect(Math.abs(r - orig[0])).toBeLessThanOrEqual(1);
      expect(Math.abs(g - orig[1])).toBeLessThanOrEqual(1);
      expect(Math.abs(b - orig[2])).toBeLessThanOrEqual(1);
    }
  });

  it("preserves greys exactly", () => {
    for (const v of [0, 64, 128, 200, 255]) {
      const [h, s, l] = rgbToHsl(v, v, v);
      const [r, g, b] = hslToRgb(h, s, l);
      expect(r).toBe(v);
      expect(g).toBe(v);
      expect(b).toBe(v);
    }
  });
});
