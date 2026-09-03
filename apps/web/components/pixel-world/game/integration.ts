// integration.ts — React-safe bridge utilities.
//
// This module is safe to import from SSR contexts. It does NOT import Phaser.
// Phaser-dependent helpers (texture loading, animation creation) live in
// integration-phaser.ts and are only imported by the Phaser scene.

import { usePixelWorldStore } from '../pixel-world-store';
import type { TileGrid } from '../pixel-tiles';
import {
  EventBus,
  EVT_FOCUS_AGENT,
  EVT_HIGHLIGHT_ROOM,
  EVT_CHAT_MESSAGE,
  EVT_WORLD_DATA_CHANGED,
} from './event-bus';

// ---------------------------------------------------------------------------
// Tilemap helpers (pure data, no Phaser)
// ---------------------------------------------------------------------------

/**
 * Convert our flat TileGrid (Int8Array layers, row-major) into the 2D number[][]
 * that Phaser.Tilemaps.Tilemap accepts for `make.tilemap({ data })`.
 *
 * Negative indices (empty) are mapped to -1 so Phaser treats them as blank.
 */
export function tileGridTo2D(tileGrid: TileGrid): number[][] {
  const ground = tileGrid.layers[0]; // Layer.GROUND
  const grid2D: number[][] = [];
  for (let r = 0; r < tileGrid.rows; r++) {
    const row: number[] = [];
    for (let c = 0; c < tileGrid.cols; c++) {
      const v = ground[r * tileGrid.cols + c];
      // Phaser data-based tilemaps treat 0 as "no tile" (empty).
      // Our Tile enum starts at 0 (VOID). Shift all values up by 1
      // so Tile.VOID=0 becomes 1 in the data, and Phaser's firstgid=1
      // maps data value 1 to tileset index 0.
      row.push(v < 0 ? 0 : v + 1);
    }
    grid2D.push(row);
  }
  return grid2D;
}

// ---------------------------------------------------------------------------
// Store sync (SSR-safe)
// ---------------------------------------------------------------------------

/**
 * Read the current Zustand store snapshot.
 * Thin wrapper so Phaser scenes don't import Zustand directly.
 */
export function getStoreState() {
  return usePixelWorldStore.getState();
}

/**
 * Subscribe to Zustand store changes. Returns unsubscribe function.
 */
export function subscribeStore(listener: (state: ReturnType<typeof getStoreState>) => void) {
  return usePixelWorldStore.subscribe(listener);
}

// ---------------------------------------------------------------------------
// EventBus convenience helpers (SSR-safe, React -> Phaser direction)
// ---------------------------------------------------------------------------

export function focusAgentFromReact(agentId: string): void {
  EventBus.emit(EVT_FOCUS_AGENT, agentId);
}

export function highlightRoomFromReact(roomId: string): void {
  EventBus.emit(EVT_HIGHLIGHT_ROOM, roomId);
}

export function sendChatMessageToPhaser(agentId: string, message: string): void {
  EventBus.emit(EVT_CHAT_MESSAGE, agentId, message);
}

export function notifyWorldDataChanged(): void {
  EventBus.emit(EVT_WORLD_DATA_CHANGED);
}
