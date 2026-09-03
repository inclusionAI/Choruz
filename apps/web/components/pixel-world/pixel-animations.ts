// pixel-animations.ts — animation state machine for pixel characters
// Pure module, no React. Used by pixel-world-store.ts to update agent
// animations each tick.

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type AnimState =
  | 'idle'
  | 'walk'
  | 'work'
  | 'think'
  // Status animations — front-only (no direction variant).
  // Played as `${agentId}_${state}` (no `_${dir}` suffix).
  // Only rendered on Choruz roster sheets (192×384). Legacy 192×192 agents
  // ignore these states and keep the sprite on the walk-row idle frame.
  | 'typing'
  | 'thinking'
  | 'sitting'
  | 'talking';
export type Direction = 'up' | 'down' | 'left' | 'right';

/** Front-only states (no directional suffix in the Phaser anim key). */
export type StatusAnimState = 'typing' | 'thinking' | 'sitting' | 'talking';
const STATUS_ANIM_STATES = new Set<AnimState>(['typing', 'thinking', 'sitting', 'talking']);
export function isStatusAnimState(state: AnimState): state is StatusAnimState {
  return STATUS_ANIM_STATES.has(state);
}

export interface AgentAnim {
  state: AnimState;
  /** Current frame index within the animation strip. */
  frame: number;
  /** How many ticks the agent has been in the current state. */
  ticksInState: number;
  /** Current pixel position. */
  x: number;
  y: number;
  /** Walk destination (only meaningful while state === 'walk'). */
  targetX: number;
  targetY: number;
  /** Optional room the agent is walking toward. */
  targetRoom: string | null;
  /** 4-direction facing for directional sprite selection. */
  direction: Direction;
  /** A* Path waypoints. Agent will move through these in order before reaching targetX/Y. */
  path?: { x: number; y: number }[];
  /** When a `walk` completes, transition to this state. Defaults to `'idle'`
   * when undefined. Set by `startWalking`'s optional arg so callers can
   * express intent like "walk to a conversation and then start typing". */
  arrivalState?: AnimState;
  /** When a transient attention state decays (currently only `thinking`),
   * transition to this state instead of the default `idle`. Used to restore
   * the at-desk pose after a mention, so a seated agent doesn't permanently
   * stand up and wander off. Narrowed to the two persistent desk states:
   * a restore to `idle` / `walk` / `talking` / `thinking` would not make
   * sense semantically. */
  resumeState?: 'typing' | 'sitting';
  /** Optional emoji bubble to pop over character */
  bubble?: string | null;
  /** Ticks remaining for the bubble to stay visible */
  bubbleTicks?: number;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/** Number of frames per animation state. */
export const FRAME_COUNTS: Record<AnimState, number> = {
  idle: 2,
  walk: 4,
  work: 4,
  think: 2,
  typing: 4,
  thinking: 4,
  sitting: 4,
  talking: 4,
};

/** Ticks per frame advance for each animation state.
 * At ~30 fps tick rate:
 *   walk = 3 ticks/frame -> ~10 FPS effective
 *   work = 2 ticks/frame -> ~15 FPS effective (bible spec: 12 FPS)
 */
export const ANIM_SPEEDS: Record<AnimState, number> = {
  idle: 12,
  walk: 3,
  work: 2,
  think: 8,
  typing: 5,
  thinking: 10,
  sitting: 15,
  talking: 6,
};

/** Pixels moved per tick while walking. */
export const WALK_SPEED = 2.0;

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/** Create a default AgentAnim at the given position. */
export function createAgentAnim(x: number, y: number): AgentAnim {
  return {
    state: 'idle',
    frame: 0,
    ticksInState: 0,
    x,
    y,
    targetX: x,
    targetY: y,
    targetRoom: null,
    direction: 'down',
  };
}

// ---------------------------------------------------------------------------
// Tick
// ---------------------------------------------------------------------------

/**
 * Advance the animation by one tick.
 *
 * - Increments `ticksInState`.
 * - Advances the frame counter at the rate defined by `ANIM_SPEEDS`.
 * - While walking, linearly interpolates position toward the target.
 *   When the remaining distance is less than `WALK_SPEED` the agent snaps
 *   to the target and transitions to `arrivalState` (set by `startWalking`)
 *   or `idle` if no arrivalState was specified.
 * - Auto-decays the short-lived `talking` gesture back to `typing`
 *   (see `TALKING_DECAY_TICKS`). Longer-lived decays (e.g., `thinking`
 *   returning to the caller's desk pose) are handled at the store level
 *   in `pixel-world-store.ts`, since they depend on per-tick inspection
 *   of a separate `resumeState` field and may vary per caller.
 * - Sets `direction` based on the dominant axis of movement (dx/dy).
 *
 * Returns a *new* AgentAnim object (immutable-friendly).
 */
/** `talking` is a short-lived gesture animation fired when an agent sends a
 * message. After this many ticks (~2s at 30fps) the agent decays to `typing`
 * — the resting "at-desk" animation — until a fresh event bumps them again. */
const TALKING_DECAY_TICKS = 60;

export function tickAgent(anim: AgentAnim): AgentAnim {
  const next: AgentAnim = { ...anim };
  next.ticksInState = anim.ticksInState + 1;

  // Auto-decay of short-lived gesture animations. Currently only talking;
  // thinking stays until a fresh event fires, matching legacy behavior.
  if (anim.state === 'talking' && next.ticksInState >= TALKING_DECAY_TICKS) {
    next.state = 'typing';
    next.frame = 0;
    next.ticksInState = 0;
    return next;
  }

  // Frame cycling
  const speed = ANIM_SPEEDS[anim.state];
  if (next.ticksInState % speed === 0) {
    next.frame = (anim.frame + 1) % FRAME_COUNTS[anim.state];
  }

  // Bubble decrement
  if (next.bubbleTicks && next.bubbleTicks > 0) {
    next.bubbleTicks -= 1;
    if (next.bubbleTicks <= 0) {
      next.bubble = null;
    }
  }

  // Walk interpolation with constant speed (Gemini improvement -- crisp pixel-art movement)
  if (anim.state === 'walk') {
    // If we have a path, walk toward the next waypoint. Otherwise, walk to final target.
    let currentTargetX = anim.targetX;
    let currentTargetY = anim.targetY;
    
    if (anim.path && anim.path.length > 0) {
      currentTargetX = anim.path[0].x;
      currentTargetY = anim.path[0].y;
    }

    const dx = currentTargetX - anim.x;
    const dy = currentTargetY - anim.y;
    const dist = Math.sqrt(dx * dx + dy * dy);

    // Update direction based on dominant movement axis
    if (dist > 0) {
      if (Math.abs(dx) > Math.abs(dy)) {
        next.direction = dx > 0 ? 'right' : 'left';
      } else {
        next.direction = dy > 0 ? 'down' : 'up';
      }
    }

    const ratio = dist > 0 ? WALK_SPEED / dist : 1;
    if (ratio >= 1) {
      // Arrive at current waypoint
      next.x = currentTargetX;
      next.y = currentTargetY;
      
      if (next.path && next.path.length > 1) {
        // Pop waypoint and continue
        next.path = next.path.slice(1);
      } else if (next.path && next.path.length === 1 && (currentTargetX !== anim.targetX || currentTargetY !== anim.targetY)) {
        // Finished path, but we still need to snap to final target
        next.path = [];
      } else {
        // Arrive at final destination. Use the caller-declared
        // `arrivalState` if present; otherwise fall back to `idle`.
        next.state = anim.arrivalState ?? 'idle';
        next.frame = 0;
        next.ticksInState = 0;
        next.path = [];
        next.arrivalState = undefined;
      }
    } else {
      // Move at constant speed
      next.x = anim.x + dx * ratio;
      next.y = anim.y + dy * ratio;
    }
  }

  return next;
}

// ---------------------------------------------------------------------------
// State transitions
// ---------------------------------------------------------------------------

export function startWalking(
  anim: AgentAnim,
  targetX: number,
  targetY: number,
  targetRoom: string | null = null,
  path?: { x: number; y: number }[],
  /** State to transition into when the walk completes. Defaults to `'idle'`
   * for generic walks; callers that know the walk is heading to a desk
   * should pass `'typing'` so the sprite sits down and starts typing on
   * arrival instead of returning to a neutral idle pose. */
  arrivalState?: AnimState,
): AgentAnim {
  let direction = anim.direction;
  const dx = targetX - anim.x;
  const dy = targetY - anim.y;

  if (Math.abs(dx) > Math.abs(dy)) {
    direction = dx > 0 ? 'right' : 'left';
  } else if (Math.abs(dy) > 0) {
    direction = dy > 0 ? 'down' : 'up';
  }

  return {
    ...anim,
    state: 'walk',
    frame: 0,
    ticksInState: 0,
    targetX,
    targetY,
    targetRoom,
    direction,
    path: path && path.length > 0 ? path : undefined,
    arrivalState,
    // Any pending "restore to desk" intent from a prior `startThinking`
    // call is discarded when the agent starts a new walk — they're on a
    // different trip now, so the old resume target no longer applies.
    resumeState: undefined,
  };
}

/** Transition to the legacy "work" state. Kept for any external caller that
 * still references it; new call sites should prefer `startTyping` (status
 * anim, actually shows the character at a desk typing). */
export function startWorking(anim: AgentAnim): AgentAnim {
  return {
    ...anim,
    state: 'work',
    frame: 0,
    ticksInState: 0,
    resumeState: undefined,
  };
}

/** Transition to the "thinking" status animation (hand on chin, head tilt).
 * For agents whose sprite sheet lacks status rows, MainScene falls back to
 * directional idle so this remains safe across roster + legacy agents.
 *
 * If the caller is in a persistent desk pose (typing / sitting) or in the
 * short-lived `talking` flash (whose base pose is always `typing`), we
 * stash the underlying desk state in `resumeState` so the later decay can
 * restore them to their seat — otherwise a mention would quietly evict
 * an agent from their desk.
 *
 * Repeated mentions while already `thinking` PRESERVE the existing
 * `resumeState`. Without this, a rapid second @mention would overwrite
 * the original seat intent with `undefined` and the agent would decay to
 * idle instead of returning to their desk. */
export function startThinking(anim: AgentAnim): AgentAnim {
  let resumeState: 'typing' | 'sitting' | undefined;
  if (anim.state === 'thinking') {
    // Already thinking — carry the existing resume intent forward so
    // repeated mentions don't leak the original desk state.
    resumeState = anim.resumeState;
  } else if (anim.state === 'typing' || anim.state === 'sitting') {
    resumeState = anim.state;
  } else if (anim.state === 'talking') {
    // `talking` is always an at-desk gesture flash (`tickAgent` decays it
    // to `typing`). Record the desk state, not the transient gesture.
    resumeState = 'typing';
  }
  return {
    ...anim,
    state: 'thinking',
    frame: 0,
    ticksInState: 0,
    resumeState,
  };
}

/** Transition to the "typing" status animation (character seated at a desk
 * typing on a laptop). Played when an agent arrives at a conversation room
 * and is working there. */
export function startTyping(anim: AgentAnim): AgentAnim {
  return {
    ...anim,
    state: 'typing',
    frame: 0,
    ticksInState: 0,
    resumeState: undefined,
  };
}

/** Transition to the "sitting" status animation (character seated at rest).
 * For use by callers that want a longer-idle "at desk" pose than `typing`. */
export function startSitting(anim: AgentAnim): AgentAnim {
  return {
    ...anim,
    state: 'sitting',
    frame: 0,
    ticksInState: 0,
    resumeState: undefined,
  };
}

/** Transition to the "talking" status animation (character gesturing with
 * hands). Short-lived — `tickAgent` auto-decays to `typing` after
 * `TALKING_DECAY_TICKS`.
 *
 * IMPORTANT CONTRACT: only call from a `typing`-capable location (desk or
 * reception_desk). The decay hard-codes `typing` as the rest state, so
 * invoking this from `sitting` / `idle` / etc. would silently drop the
 * agent into typing at the wrong spot. A dev-mode warning below flags any
 * violation so the invariant is obvious if a future caller breaks it. */
export function startTalking(anim: AgentAnim): AgentAnim {
  if (
    anim.state !== 'typing' &&
    anim.state !== 'talking' &&
    typeof process !== 'undefined' &&
    process.env?.NODE_ENV !== 'production'
  ) {
    // eslint-disable-next-line no-console
    console.warn(
      `[pixel-animations] startTalking() called from state '${anim.state}'. ` +
        `Talking auto-decays to 'typing' so callers must already be at a ` +
        `typing-capable spot (desk / reception_desk). Callers at sofa / idle / ` +
        `etc. will visibly 'type at a sofa' after decay.`,
    );
  }
  return {
    ...anim,
    state: 'talking',
    frame: 0,
    ticksInState: 0,
    resumeState: undefined,
  };
}

/** Transition to the "idle" animation. */
export function startIdle(anim: AgentAnim): AgentAnim {
  return {
    ...anim,
    state: 'idle',
    frame: 0,
    ticksInState: 0,
    resumeState: undefined,
  };
}
