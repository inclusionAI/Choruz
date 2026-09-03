import { describe, expect, it } from "vitest";
import {
  ANIM_SPEEDS,
  FRAME_COUNTS,
  WALK_SPEED,
  createAgentAnim,
  isStatusAnimState,
  startIdle,
  startSitting,
  startTalking,
  startThinking,
  startTyping,
  startWalking,
  startWorking,
  tickAgent,
} from "./pixel-animations";
import type {
  AgentAnim,
} from "./pixel-animations";

// Helpers ---------------------------------------------------------------------

function tickN(a: AgentAnim, n: number): AgentAnim {
  let cur = a;
  for (let i = 0; i < n; i++) cur = tickAgent(cur);
  return cur;
}

// isStatusAnimState -----------------------------------------------------------

describe("isStatusAnimState", () => {
  it("returns true for the 4 status states", () => {
    expect(isStatusAnimState("typing")).toBe(true);
    expect(isStatusAnimState("thinking")).toBe(true);
    expect(isStatusAnimState("sitting")).toBe(true);
    expect(isStatusAnimState("talking")).toBe(true);
  });

  it("returns false for movement / generic states", () => {
    expect(isStatusAnimState("idle")).toBe(false);
    expect(isStatusAnimState("walk")).toBe(false);
    expect(isStatusAnimState("work")).toBe(false);
    expect(isStatusAnimState("think")).toBe(false);
  });
});

// createAgentAnim -------------------------------------------------------------

describe("createAgentAnim", () => {
  it("starts in 'idle' at the given coords with target == position", () => {
    const a = createAgentAnim(100, 200);
    expect(a.state).toBe("idle");
    expect(a.x).toBe(100);
    expect(a.y).toBe(200);
    expect(a.targetX).toBe(100);
    expect(a.targetY).toBe(200);
    expect(a.frame).toBe(0);
    expect(a.ticksInState).toBe(0);
    expect(a.direction).toBe("down");
    expect(a.targetRoom).toBeNull();
  });
});

// tickAgent — basic state increments -----------------------------------------

describe("tickAgent — frame advancement", () => {
  it("advances frame at the rate defined by ANIM_SPEEDS for current state", () => {
    const idleSpeed = ANIM_SPEEDS.idle;
    let a = createAgentAnim(0, 0);
    // After exactly idleSpeed ticks, frame should advance once.
    a = tickN(a, idleSpeed);
    expect(a.frame).toBe(1);
  });

  it("frame wraps back to 0 after FRAME_COUNTS[state]", () => {
    const speed = ANIM_SPEEDS.idle;
    const frames = FRAME_COUNTS.idle;
    let a = createAgentAnim(0, 0);
    a = tickN(a, speed * frames);
    expect(a.frame).toBe(0);
  });

  it("returns a NEW object each tick (immutable)", () => {
    const a = createAgentAnim(0, 0);
    const b = tickAgent(a);
    expect(b).not.toBe(a);
    expect(a.ticksInState).toBe(0); // original untouched
    expect(b.ticksInState).toBe(1);
  });
});

// tickAgent — bubble decay ----------------------------------------------------

describe("tickAgent — bubble decay", () => {
  it("decrements bubbleTicks each tick and clears bubble at zero", () => {
    let a: AgentAnim = { ...createAgentAnim(0, 0), bubble: "💬", bubbleTicks: 2 };
    a = tickAgent(a);
    expect(a.bubbleTicks).toBe(1);
    expect(a.bubble).toBe("💬");
    a = tickAgent(a);
    expect(a.bubbleTicks).toBe(0);
    expect(a.bubble).toBeNull();
  });
});

// tickAgent — talking auto-decay ----------------------------------------------

describe("tickAgent — talking auto-decay", () => {
  it("transitions to 'typing' after TALKING_DECAY_TICKS (60) ticks", () => {
    let a = startTalking({ ...createAgentAnim(0, 0), state: "typing" });
    expect(a.state).toBe("talking");
    a = tickN(a, 60);
    expect(a.state).toBe("typing");
    expect(a.frame).toBe(0);
    expect(a.ticksInState).toBe(0);
  });

  it("does NOT decay before TALKING_DECAY_TICKS", () => {
    let a = startTalking({ ...createAgentAnim(0, 0), state: "typing" });
    a = tickN(a, 59);
    expect(a.state).toBe("talking");
  });
});

// tickAgent — walk interpolation ----------------------------------------------

describe("tickAgent — walk", () => {
  it("interpolates position toward target at WALK_SPEED per tick", () => {
    let a = startWalking(createAgentAnim(0, 0), 100, 0);
    a = tickAgent(a);
    expect(a.x).toBeCloseTo(WALK_SPEED, 5);
    expect(a.y).toBe(0);
    expect(a.state).toBe("walk");
  });

  it("snaps to target and transitions to 'idle' (default) when reaching destination", () => {
    let a = startWalking(createAgentAnim(0, 0), WALK_SPEED * 0.5, 0);
    a = tickAgent(a); // first tick should arrive (dist < WALK_SPEED)
    expect(a.state).toBe("idle");
    expect(a.x).toBeCloseTo(WALK_SPEED * 0.5, 5);
  });

  it("transitions to declared arrivalState on completion", () => {
    let a = startWalking(createAgentAnim(0, 0), WALK_SPEED * 0.5, 0, null, undefined, "typing");
    a = tickAgent(a);
    expect(a.state).toBe("typing");
  });

  it("derives direction from movement axis", () => {
    let east = tickAgent(startWalking(createAgentAnim(0, 0), 100, 0));
    expect(east.direction).toBe("right");

    let west = tickAgent(startWalking(createAgentAnim(0, 0), -100, 0));
    expect(west.direction).toBe("left");

    let north = tickAgent(startWalking(createAgentAnim(0, 0), 0, -100));
    expect(north.direction).toBe("up");

    let south = tickAgent(startWalking(createAgentAnim(0, 0), 0, 100));
    expect(south.direction).toBe("down");
  });

  it("walks the path waypoints in order before reaching the final target", () => {
    const path = [{ x: 50, y: 0 }, { x: 50, y: 50 }];
    let a = startWalking(createAgentAnim(0, 0), 100, 50, null, path);
    expect(a.path).toEqual(path);

    // Walk many ticks to traverse waypoints + final target.
    a = tickN(a, 1000);
    expect(a.state).toBe("idle");
    expect(a.x).toBeCloseTo(100, 5);
    expect(a.y).toBeCloseTo(50, 5);
  });
});

// startThinking — resumeState carry-over -------------------------------------

describe("startThinking — resumeState contract", () => {
  it("records 'typing' as resumeState when called from typing", () => {
    const a = startThinking({ ...createAgentAnim(0, 0), state: "typing" });
    expect(a.state).toBe("thinking");
    expect(a.resumeState).toBe("typing");
  });

  it("records 'sitting' as resumeState when called from sitting", () => {
    const a = startThinking({ ...createAgentAnim(0, 0), state: "sitting" });
    expect(a.resumeState).toBe("sitting");
  });

  it("treats talking as a typing-anchor for resume purposes", () => {
    const a = startThinking({ ...createAgentAnim(0, 0), state: "talking" });
    expect(a.resumeState).toBe("typing");
  });

  it("preserves resumeState across repeated mentions while already thinking", () => {
    const seated = { ...createAgentAnim(0, 0), state: "sitting" } as AgentAnim;
    const first = startThinking(seated);
    expect(first.resumeState).toBe("sitting");
    const second = startThinking(first);
    expect(second.resumeState).toBe("sitting"); // not undefined
  });

  it("leaves resumeState undefined when called from idle / walk", () => {
    const fromIdle = startThinking({ ...createAgentAnim(0, 0), state: "idle" });
    expect(fromIdle.resumeState).toBeUndefined();
    const fromWalk = startThinking({ ...createAgentAnim(0, 0), state: "walk" });
    expect(fromWalk.resumeState).toBeUndefined();
  });
});

// startWalking — discards stale resumeState ----------------------------------

describe("startWalking — clears stale resume intent", () => {
  it("a fresh walk discards any prior resumeState", () => {
    const a: AgentAnim = { ...createAgentAnim(0, 0), state: "thinking", resumeState: "sitting" };
    const b = startWalking(a, 100, 0);
    expect(b.resumeState).toBeUndefined();
  });
});

// Other start* transitions ---------------------------------------------------

describe("simple state transitions reset frame and ticks", () => {
  it("startIdle / startWorking / startTyping / startSitting all reset", () => {
    const base: AgentAnim = { ...createAgentAnim(0, 0), frame: 3, ticksInState: 47 };
    for (const fn of [startIdle, startWorking, startTyping, startSitting]) {
      const a = fn(base);
      expect(a.frame).toBe(0);
      expect(a.ticksInState).toBe(0);
    }
    expect(startIdle(base).state).toBe("idle");
    expect(startWorking(base).state).toBe("work");
    expect(startTyping(base).state).toBe("typing");
    expect(startSitting(base).state).toBe("sitting");
  });
});
