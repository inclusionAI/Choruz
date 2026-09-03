// event-bus.ts — Phaser <-> React communication bridge.
//
// Uses a lightweight custom EventEmitter (NOT Phaser.Events.EventEmitter)
// so this module can be imported from both SSR (React) and client (Phaser)
// contexts without pulling in the full Phaser library at module load time.

// ---------------------------------------------------------------------------
// Minimal typed event emitter
// ---------------------------------------------------------------------------

type Listener = (...args: any[]) => void;

class SimpleEventEmitter {
  private listeners = new Map<string, Set<Listener>>();

  on(event: string, fn: Listener): this {
    let set = this.listeners.get(event);
    if (!set) {
      set = new Set();
      this.listeners.set(event, set);
    }
    set.add(fn);
    return this;
  }

  off(event: string, fn: Listener): this {
    const set = this.listeners.get(event);
    if (set) {
      set.delete(fn);
      if (set.size === 0) this.listeners.delete(event);
    }
    return this;
  }

  emit(event: string, ...args: any[]): this {
    const set = this.listeners.get(event);
    if (set) {
      for (const fn of set) {
        fn(...args);
      }
    }
    return this;
  }

  removeAllListeners(): this {
    this.listeners.clear();
    return this;
  }
}

export const EventBus = new SimpleEventEmitter();

// ---------------------------------------------------------------------------
// Event name constants (avoids typo bugs)
// ---------------------------------------------------------------------------

/** Phaser -> React: user clicked an agent sprite. Emits the agent id only.
 * Room clicks are a separate event (EVT_ROOM_CLICKED) so telemetry can
 * answer "what did the user actually click?" without overloaded semantics. */
export const EVT_AGENT_CLICKED = 'agent-clicked';

/** Phaser -> React: user clicked a room/house rectangle. Emits the room
 * (conversation) id. Previously this piggybacked on EVT_AGENT_CLICKED
 * which made the telemetry ambiguous. */
export const EVT_ROOM_CLICKED = 'room-clicked';

/** Phaser -> React: player walked into a room. */
export const EVT_ROOM_ENTERED = 'room-entered';

/** Phaser -> React: scene finished its create() lifecycle. */
export const EVT_SCENE_READY = 'scene-ready';

/** React -> Phaser: pan camera to focus a specific agent. */
export const EVT_FOCUS_AGENT = 'focus-agent';

/** React -> Phaser: highlight a room (e.g. active conversation). */
export const EVT_HIGHLIGHT_ROOM = 'highlight-room';

/** React -> Phaser: a new chat message arrived for an agent. */
export const EVT_CHAT_MESSAGE = 'chat-message';

/** React -> Phaser: full world data refresh (conversations/agents changed). */
export const EVT_WORLD_DATA_CHANGED = 'world-data-changed';

/** React -> Phaser: destroy the scene and game instance. */
export const EVT_DESTROY = 'destroy';
