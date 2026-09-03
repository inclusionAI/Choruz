import { describe, expect, it } from "vitest";

import { stripAnsi } from "./ansi";

const ESC = "\u001b";

describe("stripAnsi", () => {
  it("removes simple SGR color codes", () => {
    expect(stripAnsi(`${ESC}[31mhello${ESC}[0m`)).toBe("hello");
  });

  it("removes 256-color and truecolor sequences", () => {
    expect(stripAnsi(`${ESC}[38;5;208mfoo${ESC}[0m`)).toBe("foo");
    expect(stripAnsi(`${ESC}[38;2;255;100;0mfoo${ESC}[0m`)).toBe("foo");
    expect(stripAnsi(`${ESC}[38:2:255:100:0mfoo${ESC}[0m`)).toBe("foo");
  });

  it("removes cursor and clear-line CSI sequences", () => {
    expect(stripAnsi(`${ESC}[2K${ESC}[Ahello`)).toBe("hello");
  });

  it("leaves text without ANSI untouched", () => {
    expect(stripAnsi("plain text")).toBe("plain text");
  });
});
