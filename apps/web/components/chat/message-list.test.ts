import { describe, expect, it } from "vitest";
import { estimateMessageHeight, messageLayoutSignature, preservePrependScrollTop, visibleRange } from "./message-list";

describe("preservePrependScrollTop", () => {
  it("keeps the same message at the same viewport position after prepending history", () => {
    expect(preservePrependScrollTop(80, 1_200, 2_000)).toBe(880);
  });

  it("never returns a negative scroll offset", () => {
    expect(preservePrependScrollTop(0, 1_200, 1_000)).toBe(0);
  });
});

describe("messageLayoutSignature", () => {
  it("changes whenever a measured row's layout inputs change", () => {
    const base = {
      containerWidth: 800,
      content: "hello",
      contentType: "text",
      attachmentMime: "",
      hasQuote: false,
      isContinuation: false,
      hasPrecedingDateSep: false,
      hasThreadRollup: false,
    };
    for (const changed of [
      { containerWidth: 600 },
      { content: "edited" },
      { contentType: "attachment" },
      { attachmentMime: "image/png" },
      { hasQuote: true },
      { isContinuation: true },
      { hasPrecedingDateSep: true },
      { hasThreadRollup: true },
    ]) {
      expect(messageLayoutSignature({ ...base, ...changed })).not.toBe(messageLayoutSignature(base));
    }
  });
});

// visibleRange ----------------------------------------------------------------
//
// `offsets[i]` is the *bottom* edge of message i (cumulative). OVERSCAN = 5 is
// applied internally on both ends, so for non-trivial inputs the range is
// always padded.

describe("visibleRange — empty / trivial", () => {
  it("returns [0,0] for an empty offsets array", () => {
    expect(visibleRange([], 0, 600)).toEqual([0, 0]);
    expect(visibleRange([], 1234, 600)).toEqual([0, 0]);
  });

  it("returns the entire array when total content is shorter than viewport", () => {
    // 3 messages, each 100px tall → cum offsets = [100, 200, 300]
    const offsets = [100, 200, 300];
    const [start, end] = visibleRange(offsets, 0, 600);
    expect(start).toBe(0);
    expect(end).toBe(3);
  });
});

describe("visibleRange — scrolling", () => {
  // 100 messages, each 50px → cum offsets = [50, 100, 150, ..., 5000]
  const offsets = Array.from({ length: 100 }, (_, i) => (i + 1) * 50);

  it("scrolled to top, viewport=300 → covers ~6 messages plus 5 overscan on each side", () => {
    const [start, end] = visibleRange(offsets, 0, 300);
    expect(start).toBe(0); // overscan can't go below 0
    // message 0..5 are visible; +5 overscan past = end >= 11
    expect(end).toBeGreaterThanOrEqual(11);
    expect(end).toBeLessThanOrEqual(13);
  });

  it("scrolled into the middle, includes the visible window", () => {
    // scroll to y=2000, viewport 500 — visible messages are roughly idx 39..49
    const [start, end] = visibleRange(offsets, 2000, 500);
    // first visible idx with offset >= scrollTop is offsets[39]=2000 → start ≤ 39
    expect(start).toBeLessThanOrEqual(39);
    // last visible idx is around 49, plus overscan
    expect(end).toBeGreaterThanOrEqual(50);
    // overscan keeps the window relatively small (~20)
    expect(end - start).toBeLessThanOrEqual(25);
  });

  it("scrolled past the end, end is clamped to total", () => {
    const [start, end] = visibleRange(offsets, 100000, 500);
    expect(end).toBe(100);
    // start should be near the tail, accounting for overscan
    expect(start).toBeGreaterThan(80);
  });
});

describe("visibleRange — invariants", () => {
  it("0 <= start <= end <= total for many random scroll positions", () => {
    const offsets = Array.from({ length: 50 }, (_, i) => (i + 1) * 30);
    const total = offsets.length;
    for (const scrollTop of [0, 50, 99, 500, 1499, 1500, 1600]) {
      for (const viewport of [100, 400, 800]) {
        const [start, end] = visibleRange(offsets, scrollTop, viewport);
        expect(start).toBeGreaterThanOrEqual(0);
        expect(end).toBeLessThanOrEqual(total);
        expect(start).toBeLessThanOrEqual(end);
      }
    }
  });

  it("keeps the rendered window bounded for a 10,000-message conversation", () => {
    const offsets = Array.from({ length: 10_000 }, (_, index) => (index + 1) * 72);
    for (const scrollTop of [0, 72_000, 360_000, 719_000]) {
      const [start, end] = visibleRange(offsets, scrollTop, 900);
      expect(end - start).toBeLessThan(40);
      expect(start).toBeGreaterThanOrEqual(0);
      expect(end).toBeLessThanOrEqual(10_000);
    }
  });
});

// estimateMessageHeight -------------------------------------------------------
//
// Pretext layout is async-loaded; in tests it stays null so the char-count
// fallback path is exercised. Constants from the implementation:
//   BUBBLE_PADDING_V = 20, TIMESTAMP_HEIGHT = 16, SENDER_HEIGHT = 20,
//   MSG_LINE_HEIGHT = 21, MSG_GROUP_GAP = 6, DATE_SEP_HEIGHT = 38,
//   SYSTEM_MSG_HEIGHT = 36, TERMINAL_BASE_HEIGHT = 34.

describe("estimateMessageHeight — system messages", () => {
  it("returns SYSTEM_MSG_HEIGHT only", () => {
    expect(
      estimateMessageHeight("m1", "x joined", 800, false, false, "system", false),
    ).toBe(36);
  });

  it("adds DATE_SEP_HEIGHT when preceded by a date separator", () => {
    expect(
      estimateMessageHeight("m1", "x joined", 800, false, false, "system", true),
    ).toBe(36 + 38);
  });
});

describe("estimateMessageHeight — runtime_transcript", () => {
  it("scales linearly with line count", () => {
    const oneLine = estimateMessageHeight(
      "m1", "single line", 800, false, false, "runtime_transcript", false,
    );
    const threeLines = estimateMessageHeight(
      "m1", "a\nb\nc", 800, false, false, "runtime_transcript", false,
    );
    expect(threeLines - oneLine).toBe(40); // 2 extra lines × 20px
  });
});

describe("estimateMessageHeight — attachments", () => {
  it("reserves stable media space before lazy images load", () => {
    const image = estimateMessageHeight(
      "image-1", "photo.png", 800, false, false, "attachment", false, false, "image/png",
    );
    const file = estimateMessageHeight(
      "file-1", "notes.txt", 800, false, false, "attachment", false, false, "text/plain",
    );
    expect(image).toBeGreaterThan(file);
    expect(image).toBeGreaterThanOrEqual(300);
  });

  it("accounts for grouping, quotes, and thread rollups", () => {
    const base = estimateMessageHeight(
      "file-2", "notes.txt", 800, false, false, "attachment", false, false, "text/plain",
    );
    const decorated = estimateMessageHeight(
      "file-2", "notes.txt", 800, true, true, "attachment", false, true, "text/plain",
    );
    expect(decorated - base).toBe(40 - 20 + 32);
  });
});

describe("estimateMessageHeight — text messages", () => {
  it("non-continuation includes sender row (+20px) over a continuation", () => {
    const cont = estimateMessageHeight("m1", "hi", 800, false, true, "text", false);
    const head = estimateMessageHeight("m1", "hi", 800, false, false, "text", false);
    expect(head - cont).toBe(20);
  });

  it("hasQuote adds QUOTE_BLOCK_HEIGHT (+40px)", () => {
    const noQuote = estimateMessageHeight("m1", "hi", 800, false, false, "text", false);
    const withQuote = estimateMessageHeight("m1", "hi", 800, true, false, "text", false);
    expect(withQuote - noQuote).toBe(40);
  });

  it("longer content takes more vertical space (line-wrapping)", () => {
    const short = estimateMessageHeight("m1", "hi", 800, false, false, "text", false);
    const long = estimateMessageHeight(
      "m2",
      "x".repeat(2000),
      800,
      false,
      false,
      "text",
      false,
    );
    expect(long).toBeGreaterThan(short);
  });

  it("returns at least one line of height even for empty content", () => {
    const h = estimateMessageHeight("m1", "", 800, false, false, "text", false);
    // padding + 1 line + timestamp + sender + group gap = 20+21+16+20+6 = 83
    expect(h).toBeGreaterThanOrEqual(83);
  });

  it("does not crash for zero-width container (degenerate case)", () => {
    expect(() =>
      estimateMessageHeight("m1", "hi", 0, false, false, "text", false),
    ).not.toThrow();
  });
});
