'use client';

import { useRef, useEffect, useCallback, useState } from 'react';
import { trace } from '../../lib/api/choruz-trace';
import { usePixelWorldStore } from './pixel-world-store';

/** Attach the active store `instanceId` to any pixel-world trace payload. */
function pixEvent(name: string, data?: Record<string, unknown>): void {
  const instanceId = usePixelWorldStore.getState().instanceId;
  trace.event(name, { pixel_world_instance_id: instanceId, ...(data ?? {}) });
}
function pixSpan(name: string, data?: Record<string, unknown>) {
  const instanceId = usePixelWorldStore.getState().instanceId;
  return trace.start(name, { pixel_world_instance_id: instanceId, ...(data ?? {}) });
}
import {
  EventBus,
  EVT_AGENT_CLICKED,
  EVT_ROOM_CLICKED,
  EVT_ROOM_ENTERED,
  EVT_SCENE_READY,
} from './game/event-bus';
import {
  highlightRoomFromReact,
  notifyWorldDataChanged,
} from './game/integration';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface PixelWorldProps {
  conversations: any[];
  agents: any[];
  messagesByConv: Record<string, any[]>;
  activeConvId: string | null;
  onSelectConversation: (convId: string) => void;
  onClose: () => void;
}

// ---------------------------------------------------------------------------
// Phaser dynamic import (SSR disabled)
// ---------------------------------------------------------------------------

let phaserPromise: Promise<typeof import('phaser')> | null = null;

function getPhaserModule() {
  if (!phaserPromise) {
    phaserPromise = import('phaser');
  }
  return phaserPromise;
}

function buildWorldSignature(conversations: any[], agentsList: any[]): string {
  const groupSignature = conversations
    .filter((c: any) => c.conversation_type === 'group' || c.type === 'group')
    .map((conv: any) => {
      const members = Array.isArray(conv.members)
        ? conv.members
        : typeof conv.members === 'object' && conv.members
          ? Object.keys(conv.members)
          : [];
      return {
        id: conv.id ?? conv.conversation_id,
        name: conv.name ?? '',
        members: [...members].sort(),
      };
    })
    .sort((a, b) => String(a.id).localeCompare(String(b.id)));

  const agentSignature = agentsList
    .map((agent: any) => ({
      id: agent.id ?? agent.agent_id,
      name: agent.name ?? agent.display_name ?? '',
    }))
    .sort((a, b) => String(a.id).localeCompare(String(b.id)));

  return JSON.stringify({ groups: groupSignature, agents: agentSignature });
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export default function PixelWorld({
  conversations,
  agents: agentsList,
  messagesByConv,
  activeConvId,
  onSelectConversation,
  onClose,
}: PixelWorldProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const gameRef = useRef<import('phaser').Game | null>(null);
  const sceneReadyRef = useRef(false);
  const lastWorldSignatureRef = useRef<string | null>(null);

  // Panel resize & fullscreen state
  const [panelWidth, setPanelWidth] = useState<number | null>(null);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const resizing = useRef(false);

  // Zustand store actions ref (avoids re-render subscription)
  const storeActions = useRef(usePixelWorldStore.getState());
  storeActions.current = usePixelWorldStore.getState();

  // ─── Initialize store on mount / when data changes ───────────────────────
  useEffect(() => {
    const worldSignature = buildWorldSignature(conversations, agentsList);
    if (lastWorldSignatureRef.current === worldSignature) {
      pixEvent('pixel_world_skip_reinit', {
        conv_count: conversations.length,
        agent_count: agentsList.length,
      });
      return;
    }
    lastWorldSignatureRef.current = worldSignature;

    const span = pixSpan('pixel_world_init', {
      conv_count: conversations.length,
      agent_count: agentsList.length,
    });
    try {
      storeActions.current.initialize(conversations, agentsList, messagesByConv);
      const s = usePixelWorldStore.getState();
      span.end({
        houses: s.houses.size,
        agents: s.agents.size,
        // The store just freshly generated this; include it so consumers know
        // which session everything that follows belongs to.
        pixel_world_instance_id: s.instanceId,
      });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      span.end({ error: msg });
      pixEvent('pixel_world_init_failed', { error: msg });
      throw err;
    }

    // Notify Phaser scene that data changed (if scene is already running)
    if (sceneReadyRef.current) {
      notifyWorldDataChanged();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conversations, agentsList, messagesByConv]);

  // ─── Auto-focus when active conversation changes ─────────────────────────
  useEffect(() => {
    if (activeConvId && sceneReadyRef.current) {
      highlightRoomFromReact(activeConvId);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeConvId]);

  // ─── Mount Phaser game ───────────────────────────────────────────────────
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    let game: import('phaser').Game | null = null;
    let destroyed = false;

    const boot = async () => {
      // Wait for store to have data before booting Phaser
      // (store.initialize runs in a separate useEffect that may fire first or after).
      // The timeout guards against init failures that would otherwise hang
      // forever — previously manifested as "Phaser never boots" with no log.
      const BOOT_STORE_TIMEOUT_MS = 10_000;
      const waitStart = Date.now();
      const waitForStore = () => new Promise<boolean>((resolve) => {
        const check = () => {
          if (destroyed) return resolve(false);
          const s = usePixelWorldStore.getState();
          if (s.tileGrid) return resolve(true);
          if (Date.now() - waitStart >= BOOT_STORE_TIMEOUT_MS) return resolve(false);
          setTimeout(check, 50);
        };
        check();
      });

      const ready = await waitForStore();
      if (destroyed) return;
      if (!ready) {
        pixEvent('pixel_world_boot_timeout', {
          waited_ms: Date.now() - waitStart,
          reason: 'tile_grid_never_ready',
        });
        return;
      }

      // Dynamic import keeps Phaser out of SSR bundle
      await getPhaserModule();
      const { createGameConfig } = await import('./game/config');

      if (destroyed) return;

      const config = createGameConfig(container);
      game = new (await getPhaserModule()).Game(config);
      gameRef.current = game;

      // Auto-focus the canvas so keyboard input (WASD) works immediately
      // without requiring the user to click the canvas first.
      setTimeout(() => {
        const canvas = document.querySelector('.pixel-world-panel canvas') as HTMLCanvasElement;
        if (canvas) {
          canvas.setAttribute('tabindex', '0');
          canvas.focus();
        }
      }, 500);

      pixEvent('pixel_world_phaser_boot', {
        boot_wait_ms: Date.now() - waitStart,
        renderer: (config as any).type,
      });
    };

    boot().catch((err) => {
      const msg = err instanceof Error ? err.message : String(err);
      console.error('[PixelWorld] Failed to boot Phaser:', err);
      pixEvent('pixel_world_boot_failed', {
        error: msg,
        stack: err instanceof Error ? err.stack?.slice(0, 4000) : undefined,
      });
    });

    return () => {
      destroyed = true;
      if (game) {
        game.destroy(true);
        gameRef.current = null;
      }
      sceneReadyRef.current = false;
    };
  }, []);

  // ─── EventBus listeners (Phaser -> React) ────────────────────────────────
  useEffect(() => {
    const onSceneReady = () => {
      sceneReadyRef.current = true;
      pixEvent('pixel_world_scene_ready');

      // If there's an active conversation, highlight it now
      if (activeConvId) {
        highlightRoomFromReact(activeConvId);
      }
    };

    const onAgentClicked = (payload: string | { agent_id: string; room_id: string | null }) => {
      // Accept both legacy (string id) and the new structured payload so
      // tests / older emitters keep working. MainScene now always sends the
      // object form with room context; keeping the id-only branch avoids
      // touching unrelated callers (e.g. integration.ts).
      const agentId = typeof payload === 'string' ? payload : payload.agent_id;
      const roomId = typeof payload === 'string' ? null : payload.room_id;
      pixEvent('pixel_world_agent_clicked', {
        agent_id: agentId,
        room_id: roomId,
      });
      // Single selection per click — we used to also fire from the room
      // handler, which caused two `onSelectConversation` calls for one
      // physical click and raced SWR.
      onSelectConversation(agentId);
    };

    const onRoomClicked = (roomId: string) => {
      pixEvent('pixel_world_room_clicked', { room_id: roomId });
      onSelectConversation(roomId);
    };

    const onRoomEntered = (roomId: string) => {
      pixEvent('pixel_world_room_entered', { room_id: roomId });
    };

    EventBus.on(EVT_SCENE_READY, onSceneReady);
    EventBus.on(EVT_AGENT_CLICKED, onAgentClicked);
    EventBus.on(EVT_ROOM_CLICKED, onRoomClicked);
    EventBus.on(EVT_ROOM_ENTERED, onRoomEntered);

    return () => {
      EventBus.off(EVT_SCENE_READY, onSceneReady);
      EventBus.off(EVT_AGENT_CLICKED, onAgentClicked);
      EventBus.off(EVT_ROOM_CLICKED, onRoomClicked);
      EventBus.off(EVT_ROOM_ENTERED, onRoomEntered);
    };
  }, [activeConvId, onSelectConversation]);

  // ─── Handle container resize -> Phaser scale ────────────────────────────
  // The observer is set up unconditionally on the container div.
  // gameRef.current is checked inside the callback (it becomes non-null
  // after the async boot completes).
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const observer = new ResizeObserver(() => {
      if (gameRef.current && container) {
        const { width, height } = container.getBoundingClientRect();
        if (width > 0 && height > 0) {
          gameRef.current.scale.resize(width, height);
        }
      }
    });

    observer.observe(container);

    return () => observer.disconnect();
  }, []);

  // ─── Panel drag-resize handler ──────────────────────────────────────────
  const handleResizeMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    resizing.current = true;
    const startX = e.clientX;
    const startWidth = panelRef.current?.offsetWidth ?? 600;
    let finalWidth = startWidth;

    const onMove = (ev: MouseEvent) => {
      if (!resizing.current) return;
      const delta = startX - ev.clientX;
      const newW = Math.max(300, Math.min(window.innerWidth, startWidth + delta));
      finalWidth = newW;
      setPanelWidth(newW);
    };
    const onUp = () => {
      resizing.current = false;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      if (finalWidth !== startWidth) {
        pixEvent('pixel_world_resized', {
          width_before: startWidth,
          width_after: finalWidth,
        });
      }
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }, []);

  // ─── Render ──────────────────────────────────────────────────────────────

  return (
    <div
      className={`pixel-world-panel${isFullscreen ? ' fullscreen' : ''}`}
      ref={panelRef}
      style={!isFullscreen && panelWidth ? { width: panelWidth } : undefined}
      onClick={() => {
        // Re-focus canvas on any click inside the panel so WASD keeps working
        // even after the user clicked elsewhere (sidebar, chat, etc.)
        const canvas = panelRef.current?.querySelector('canvas') as HTMLCanvasElement | null;
        if (canvas) canvas.focus();
      }}
    >
      {/* Drag handle on left edge */}
      {!isFullscreen && (
        <div className="pixel-world-resize-handle" onMouseDown={handleResizeMouseDown} />
      )}

      <div className="pixel-world-header">
        <span>Pixel World</span>
        <div className="pixel-world-header-actions">
          <button
            className="pixel-world-icon-btn"
            onClick={() => {
              const next = !isFullscreen;
              pixEvent('pixel_world_fullscreen_toggled', { is_fullscreen: next });
              setIsFullscreen(next);
            }}
            aria-label={isFullscreen ? 'Exit fullscreen' : 'Fullscreen'}
            title={isFullscreen ? 'Exit fullscreen' : 'Fullscreen'}
          >
            {isFullscreen ? '\u229e' : '\u229e'}
          </button>
          <button
            onClick={() => {
              pixEvent('pixel_world_closed', { via: 'x_button' });
              onClose();
            }}
            aria-label="Close Pixel World"
            className="pixel-world-close-btn"
          >
            X
          </button>
        </div>
      </div>

      {/* Phaser mounts into this container div */}
      <div
        ref={containerRef}
        className="pixel-world-canvas"
        style={{
          width: '100%',
          height: '100%',
          display: 'block',
          touchAction: 'none',
          overflow: 'hidden',
        }}
      />
    </div>
  );
}
