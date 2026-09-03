import { describe, expect, it } from "vitest";

import { createTerminalWriteBuffer } from "./terminal-write-buffer";

function scheduler() {
  const callbacks: FrameRequestCallback[] = [];
  return {
    callbacks,
    schedule(callback: FrameRequestCallback) {
      callbacks.push(callback);
      return callbacks.length;
    },
    cancel() {},
    runAll() {
      while (callbacks.length > 0) callbacks.shift()!(0);
    },
  };
}

describe("terminal write buffering", () => {
  it("preserves order while splitting a multi-megabyte burst by frame budget", () => {
    const frames = scheduler();
    const writes: string[] = [];
    const buffer = createTerminalWriteBuffer(
      (data) => writes.push(data),
      frames.schedule,
      frames.cancel,
      64 * 1024,
      6 * 1024 * 1024,
    );
    const chunk = "x".repeat(1024 * 1024);
    for (let i = 0; i < 5; i += 1) buffer.enqueue(`${i}${chunk}`);

    expect(frames.callbacks).toHaveLength(1);
    frames.runAll();
    expect(writes.length).toBeGreaterThan(5);
    expect(writes.join("")).toBe(Array.from({ length: 5 }, (_, i) => `${i}${chunk}`).join(""));
    expect(buffer.pendingCharacters()).toBe(0);
  });

  it("bounds a producer that outruns rendering and reports truncation", () => {
    const frames = scheduler();
    const writes: string[] = [];
    const buffer = createTerminalWriteBuffer(
      (data) => writes.push(data),
      frames.schedule,
      frames.cancel,
      8,
      16,
    );
    buffer.enqueue("0123456789abcdefghijklmnop");
    frames.runAll();

    expect(writes.join("")).toContain("output truncated during burst");
    expect(writes.join("")).toContain("abcdefghijklmnop");
    expect(buffer.pendingCharacters()).toBe(0);
  });

  it("does not carry incomplete binary decoding across a string frame", () => {
    const frames = scheduler();
    const writes: string[] = [];
    const buffer = createTerminalWriteBuffer(
      (data) => writes.push(data),
      frames.schedule,
      frames.cancel,
    );
    buffer.enqueue(new Uint8Array([0xe2, 0x82]));
    buffer.enqueue("X");
    buffer.enqueue(new Uint8Array([0xac]));
    frames.runAll();

    expect(writes.join("")).toBe("\ufffdX\ufffd");
  });
});
