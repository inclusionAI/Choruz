import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { formatChatDivider, formatAbsoluteTime, shouldShowTimeDivider } from "./format-chat-time";

// Anchor "now" so day-diff math is deterministic. Picked a Wednesday so weekday
// branches are exercised.
const NOW = new Date(2026, 4, 13, 14, 30, 0); // 2026-05-13 14:30 local

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("formatChatDivider — same day", () => {
  it("returns HH:MM only for same-day timestamps", () => {
    const timestamp = new Date(2026, 4, 13, 9, 5, 0).toISOString();
    expect(formatChatDivider(timestamp, NOW)).toBe("09:05");
  });

  it("pads hour and minute to two digits", () => {
    const timestamp = new Date(2026, 4, 13, 0, 0, 0).toISOString();
    expect(formatChatDivider(timestamp, NOW)).toBe("00:00");
  });
});

describe("formatChatDivider — yesterday", () => {
  it("uses the English Yesterday prefix", () => {
    const timestamp = new Date(2026, 4, 12, 23, 59, 0).toISOString();
    expect(formatChatDivider(timestamp, NOW)).toBe("Yesterday 23:59");
  });
});

describe("formatChatDivider — within seven days", () => {
  it("uses an English abbreviated weekday", () => {
    const timestamp = new Date(2026, 4, 10, 12, 0, 0).toISOString();
    expect(formatChatDivider(timestamp, NOW)).toBe("Sun 12:00");
  });
});

describe("formatChatDivider — same year", () => {
  it("uses an English abbreviated month", () => {
    const timestamp = new Date(2026, 0, 15, 10, 0, 0).toISOString();
    expect(formatChatDivider(timestamp, NOW)).toBe("Jan 15 10:00");
  });
});

describe("formatChatDivider — different year", () => {
  it("includes the year in English order", () => {
    const timestamp = new Date(2024, 11, 1, 9, 0, 0).toISOString();
    expect(formatChatDivider(timestamp, NOW)).toBe("Dec 1, 2024 09:00");
  });
});

describe("shouldShowTimeDivider", () => {
  it("returns true when there is no previous message", () => {
    expect(shouldShowTimeDivider(null, "2026-05-13T10:00:00.000Z")).toBe(true);
  });

  it("returns false when the gap is under five minutes on the same day", () => {
    const previous = new Date(2026, 4, 13, 10, 0, 0).toISOString();
    const current = new Date(2026, 4, 13, 10, 4, 30).toISOString();
    expect(shouldShowTimeDivider(previous, current)).toBe(false);
  });

  it("returns true when the gap is at least five minutes", () => {
    const previous = new Date(2026, 4, 13, 10, 0, 0).toISOString();
    const current = new Date(2026, 4, 13, 10, 5, 0).toISOString();
    expect(shouldShowTimeDivider(previous, current)).toBe(true);
  });

  it("returns true across a day boundary even when the gap is under five minutes", () => {
    const previous = new Date(2026, 4, 12, 23, 58, 0).toISOString();
    const current = new Date(2026, 4, 13, 0, 1, 0).toISOString();
    expect(shouldShowTimeDivider(previous, current)).toBe(true);
  });
});

describe("formatAbsoluteTime", () => {
  it("returns an English date and time with seconds", () => {
    const timestamp = new Date(2026, 4, 13, 14, 30, 45).toISOString();
    expect(formatAbsoluteTime(timestamp)).toMatch(/^5\/13\/2026, 02:30:45 PM$/);
  });
});
