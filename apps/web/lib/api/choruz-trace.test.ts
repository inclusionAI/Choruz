import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  beginTrace,
  currentTraceId,
  endTrace,
  trace,
  traceRing,
} from "./choruz-trace";

// Tests share a module-global ring + active-id state. Use fake timers so the
// 10-second TTL doesn't asynchronously evict ids during tests, and snapshot
// the ring length so we can assert relative pushes.

beforeEach(() => {
  vi.useFakeTimers();
  // Force-clear any active trace from a prior test.
  endTrace();
});

afterEach(() => {
  endTrace();
  vi.useRealTimers();
});

// beginTrace / endTrace / currentTraceId -------------------------------------

describe("beginTrace / endTrace / currentTraceId", () => {
  it("beginTrace returns a non-empty hex string and sets the active id", () => {
    const id = beginTrace();
    expect(id).toMatch(/^[0-9a-f]+$/);
    expect(id.length).toBeGreaterThan(0);
    expect(currentTraceId()).toBe(id);
  });

  it("two beginTrace calls produce different ids and replace each other", () => {
    const a = beginTrace();
    const b = beginTrace();
    expect(b).not.toBe(a);
    expect(currentTraceId()).toBe(b);
  });

  it("endTrace() clears the active trace id", () => {
    beginTrace();
    expect(currentTraceId()).not.toBeNull();
    endTrace();
    expect(currentTraceId()).toBeNull();
  });

  it("endTrace(expectedId) is a no-op if active id no longer matches", () => {
    const a = beginTrace();
    const b = beginTrace(); // active is now b
    endTrace(a); // tries to end the OLD id — should not clear
    expect(currentTraceId()).toBe(b);
  });

  it("endTrace(expectedId) clears when ids match", () => {
    const a = beginTrace();
    endTrace(a);
    expect(currentTraceId()).toBeNull();
  });

  it("active trace id auto-evicts after 10 seconds without activity", () => {
    beginTrace();
    expect(currentTraceId()).not.toBeNull();
    vi.advanceTimersByTime(9_999);
    expect(currentTraceId()).not.toBeNull(); // still alive
    vi.advanceTimersByTime(2);
    expect(currentTraceId()).toBeNull(); // evicted
  });
});

// trace.event auto-begins ----------------------------------------------------

describe("trace.event", () => {
  it("auto-creates an active trace id when none exists", () => {
    expect(currentTraceId()).toBeNull();
    trace.event("test_event", { foo: 1 });
    expect(currentTraceId()).not.toBeNull();
  });

  it("attaches the event to the active trace id", () => {
    beginTrace();
    const tid = currentTraceId();
    const before = traceRing().length;
    trace.event("test_event_2");
    const ring = traceRing();
    expect(ring.length).toBe(before + 1);
    const newest = ring[ring.length - 1];
    expect(newest.traceId).toBe(tid);
    expect(newest.name).toBe("test_event_2");
    expect(newest.source).toBe("FE");
  });

  it("does not extend the active trace TTL on background activity", () => {
    beginTrace();
    vi.advanceTimersByTime(9_000);
    trace.event("background_tick"); // would extend TTL if buggy
    vi.advanceTimersByTime(1_500);
    // 10.5s total since beginTrace → should be evicted because the event
    // does NOT reset the timer.
    expect(currentTraceId()).toBeNull();
  });
});

// trace.start / span.end -----------------------------------------------------

describe("trace.start + span.end", () => {
  it("returns a span whose end() pushes a complete entry to the ring with durationMs", () => {
    const before = traceRing().length;
    const span = trace.start("op_x", { initial: true });
    span.end({ extra: 42 });
    const ring = traceRing();
    expect(ring.length).toBe(before + 1);
    const entry = ring[ring.length - 1];
    expect(entry.name).toBe("op_x");
    expect(typeof entry.durationMs).toBe("number");
    expect(entry.durationMs).toBeGreaterThanOrEqual(0);
    expect(entry.data).toMatchObject({ initial: true, extra: 42 });
  });

  it("merges start data and end extras (extras override on collision)", () => {
    const span = trace.start("op_y", { v: "start" });
    span.end({ v: "end" });
    const entry = traceRing()[traceRing().length - 1];
    expect(entry.data?.v).toBe("end");
  });

  it("redacts sensitive span data before it reaches the ring", () => {
    const span = trace.start("config_save", {
      workspacePath: "/Users/alice/private/workspace",
      attachment: { fileName: "private-plan.txt" },
    });
    span.end({ filePath: "/tmp/private.txt" });
    const entry = traceRing()[traceRing().length - 1];
    const serialized = JSON.stringify(entry);
    expect(serialized).not.toContain("/Users/alice/private/workspace");
    expect(serialized).not.toContain("private-plan.txt");
    expect(serialized).not.toContain("/tmp/private.txt");
    expect(entry.data).toMatchObject({
      workspacePath: "[REDACTED]",
      filePath: "[REDACTED]",
      attachment: { fileName: "[REDACTED]" },
    });
  });

  it("redacts marker-shaped secrets in generic error fields before queueing", () => {
    const before = traceRing().length;
    trace.event("unhandled_rejection", {
      reason: "request failed with Bearer real-token-value",
      error: "secret:secret-test-value token=token-test-value",
    });
    const entry = traceRing()[before];
    const serialized = JSON.stringify(entry);
    expect(serialized).not.toContain("real-token-value");
    expect(serialized).not.toContain("secret-test-value");
    expect(serialized).not.toContain("token-test-value");
    expect(entry.data).toMatchObject({
      reason: "request failed with Bearer [REDACTED]",
      error: "secret:[REDACTED] token=[REDACTED]",
    });
  });

  it("sets data to undefined when both start data and end extras are empty", () => {
    const span = trace.start("op_z");
    span.end();
    const entry = traceRing()[traceRing().length - 1];
    expect(entry.data).toBeUndefined();
  });

  it("returned span carries the active traceId", () => {
    const tid = beginTrace();
    const span = trace.start("op_a");
    expect(span.traceId).toBe(tid);
    span.end();
  });
});

// Ring buffer cap ------------------------------------------------------------

describe("traceRing", () => {
  it("returns a snapshot view of recorded entries (event entries appear)", () => {
    const before = traceRing().length;
    trace.event("ring_smoke_test");
    expect(traceRing().length).toBe(before + 1);
  });
});
