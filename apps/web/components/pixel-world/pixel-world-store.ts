// pixel-world-store.ts — Zustand store connecting real-time events to pixel world state.
// Bridges conversation/agent data with the pixel-art rendering layer.
// Office building theme: rooms connected by hallways via spiral packer layout.

import { create } from 'zustand';

import { trace } from '../../lib/api/choruz-trace';
import { generateSprite, hashStr, type PixelSprite } from './pixel-sprites';
import { generateHouse, clearRoomCache, type GeneratedHouse } from './pixel-houses';
import {
  type AgentAnim,
  type AnimState,
  tickAgent,
  startWalking,
  startTalking,
  startThinking,
  createAgentAnim,
} from './pixel-animations';
import {
  type TileGrid,
  type ChunkCache,
  type OfficeRoom,
  type OfficeLayout,
  TILE_SIZE,
  generateWorldGrid,
  createChunkCache,
  loadTileSheets,
} from './pixel-tiles';
// invalidateLightmap was in the old canvas renderer (pixel-renderer.ts).
// With the Phaser migration, the renderer is gone; this is now a no-op stub.
function invalidateLightmap(): void { /* no-op — Phaser handles rendering */ }

// ---------------------------------------------------------------------------
// Telemetry helpers — every pixel-world event must carry the instance id so
// reloads / remounts do not collapse into one session in log storage.
// ---------------------------------------------------------------------------

function newPixelInstanceId(): string {
  const g = globalThis as any;
  if (g.crypto && typeof g.crypto.randomUUID === 'function') {
    return g.crypto.randomUUID();
  }
  // Fallback for older runtimes (e.g. jsdom in tests).
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

/** Build the base payload attached to every pixel-world trace event. Callers
 * spread their own fields on top. Fields are intentionally lowercase /
 * snake_case so logs aggregate cleanly across FE/BE. */
function pixTraceBase(instanceId: string | null): { pixel_world_instance_id: string | null } {
  return { pixel_world_instance_id: instanceId };
}

/** Emit a pixel-world event with instance id auto-attached. */
function emitPix(
  instanceId: string | null,
  name: string,
  data?: Record<string, unknown>,
): void {
  trace.event(name, { ...pixTraceBase(instanceId), ...(data ?? {}) });
}

/** Public helper for non-store call sites (sidebar menu, chat-app poll
 * fallback, etc.) so every `pixel_world_*` event is stamped with the active
 * `instance_id` without each caller duplicating the store lookup.
 *
 * Lazy-init invariant: if the event fires before `initialize()` has run
 * (typical for `pixel_world_menu_clicked` / `pixel_world_opened` — the user
 * hasn't even opened Pixel World yet, so the store has no id), we generate
 * a placeholder id here and stash it so *subsequent* pre-init events share
 * the same id. `initialize()` will overwrite with a fresh id once data is
 * ready — that boundary is itself a useful signal ("session id changed
 * from X to Y means the user just opened Pixel World"). */
export function emitPixelWorldEvent(
  name: string,
  data?: Record<string, unknown>,
): void {
  let instanceId = usePixelWorldStore.getState().instanceId;
  if (instanceId === null) {
    instanceId = `pre-init-${newPixelInstanceId()}`;
    usePixelWorldStore.setState({ instanceId });
  }
  emitPix(instanceId, name, data);
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface HouseInfo {
  id: string;           // conversation_id
  name: string;         // group name
  house: GeneratedHouse;
  worldX: number;       // position in world pixels
  worldY: number;
  members: string[];    // agent IDs
  room: OfficeRoom;     // office room definition
}

export interface PixelAgentState {
  id: string;
  name: string;
  sprite: PixelSprite;
  anim: AgentAnim;
  currentHouseId: string | null;
  /** Stable "home" anchor in world coords. Used as the wander center so an
   *  agent always drifts back to its workspace after replying to a message,
   *  instead of random-walking off across the floor. Set on first spawn,
   *  preserved across `initialize()` re-runs. */
  homeX: number;
  homeY: number;
}

/** Particle effect — dust when walking, ambient effects. */
export interface Particle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number;
  maxLife: number;
  size: number;
  color: string;
  type: 'dust' | 'leaf' | 'sparks' | 'bubbles' | 'spores' | 'embers' | 'glitch' | 'ice' | 'hearts' | 'dust_mote' | 'steam' | 'water_bubble';
  /** Alpha override for particles that manage their own opacity. */
  alpha?: number;
}

export interface PlayerState {
  x: number;
  y: number;
  direction: 'up' | 'down' | 'left' | 'right';
  currentRoomId: string | null;
  sprite: PixelSprite;
}

/** Raw walkability mask sampled from the floor PNG (white = walkable).
 *  Pixel buffer is in scene-pixel coordinates (matches floorN_mask.png at
 *  native resolution). Pathfinding converts world↔scene coords on the fly
 *  using `worldWidth/worldHeight` from the same store. */
export interface WalkabilityMask {
  pixels: Uint8ClampedArray;
  width: number;
  height: number;
}

export interface PixelWorldState {
  agents: Map<string, PixelAgentState>;
  houses: Map<string, HouseInfo>;
  worldWidth: number;
  worldHeight: number;
  cameraX: number;
  cameraY: number;
  targetCameraX: number;
  targetCameraY: number;
  zoom: number;
  targetZoom: number;
  player: PlayerState | null;
  particles: Particle[];
  timeOfDay: number;
  isOpen: boolean;
  tileGrid: TileGrid | null;
  /** Walkability mask sampled from the active floor's PNG (set by
   *  MainScene after the texture loads). Used for agent A* pathfinding so
   *  agents respect the same visible walls the player does. */
  walkabilityMask: WalkabilityMask | null;
  chunkCache: ChunkCache | null;
  frameCount: number;
  /** Random id regenerated on every `initialize()` run. Attached to every
   * pixel-world telemetry event so a reload / remount does not get conflated
   * with the previous session in the log stream. `null` until first init. */
  instanceId: string | null;

  initialize: (
    conversations: any[],
    agentsList: any[],
    messagesByConv: Record<string, any[]>,
  ) => void;
  handleMessage: (senderId: string, conversationId: string) => void;
  handleMention: (agentId: string) => void;
  /** Move an agent to a random interaction point within their current room.
   * Distinct from `handleMessage`: no "talking" flip on same-room calls,
   * and the arrival state is `idle` rather than `typing`. Intended for the
   * ambient NPC wander loop. */
  wanderAgentInRoom: (agentId: string) => void;
  tick: (displayWidth?: number, displayHeight?: number) => void;
  togglePanel: () => void;
  setCameraPosition: (x: number, y: number) => void;
  focusHouse: (houseId: string) => void;
  setZoom: (zoom: number) => void;
  panCamera: (dx: number, dy: number) => void;
  centerCamera: () => void;
  movePlayer: (dx: number, dy: number) => void;
  updatePlayerRoom: () => void;
  setWalkabilityMask: (pixels: Uint8ClampedArray, width: number, height: number) => void;
}

// ---------------------------------------------------------------------------
// Layout constants
// ---------------------------------------------------------------------------

const CAMERA_LERP = 0.12;
const MIN_ZOOM = 1;
const MAX_ZOOM = 4;

/** Dust particle colors — used for all human walking particles. */
const DUST_COLORS = ['#C8B89A', '#D4C4A8', '#B5A68C', '#8B7355'];

const FLOOR1_SCENE_WIDTH = 2158;
const FLOOR1_SCENE_HEIGHT = 1984;
const FLOOR1_AGENT_SPAWN_ZONES_SCENE = [
  { x: 95, y: 1525, width: 600, height: 200 },
  { x: 1020, y: 1480, width: 780, height: 380 },
  { x: 90, y: 900, width: 640, height: 405 },
];

const AGENT_MASK_SAFE_RADIUS_PX = 10;

function sceneToWorldPoint(sceneX: number, sceneY: number, worldWidth: number, worldHeight: number): { x: number; y: number } {
  return {
    x: (sceneX / FLOOR1_SCENE_WIDTH) * worldWidth,
    y: (sceneY / FLOOR1_SCENE_HEIGHT) * worldHeight,
  };
}

function isSafeWalkableMaskPoint(mask: WalkabilityMask, sceneX: number, sceneY: number, radius = AGENT_MASK_SAFE_RADIUS_PX): boolean {
  const samplePoints: Array<[number, number]> = [
    [sceneX, sceneY],
    [sceneX - radius, sceneY],
    [sceneX + radius, sceneY],
    [sceneX, sceneY - radius],
    [sceneX, sceneY + radius],
    [sceneX - radius, sceneY - radius],
    [sceneX + radius, sceneY - radius],
    [sceneX - radius, sceneY + radius],
    [sceneX + radius, sceneY + radius],
  ];

  for (const [sx, sy] of samplePoints) {
    const px = Math.round(sx);
    const py = Math.round(sy);
    if (px < 0 || px >= mask.width || py < 0 || py >= mask.height) return false;
    const idx = (py * mask.width + px) * 4;
    const a = mask.pixels[idx + 3];
    if (a < 16) return false;
    const r = mask.pixels[idx];
    const g = mask.pixels[idx + 1];
    const b = mask.pixels[idx + 2];
    if (!(r > 240 && g > 240 && b > 240)) return false;
  }

  return true;
}

function pickInitialAgentSpawnWorld(
  agentId: string,
  worldWidth: number,
  worldHeight: number,
  mask?: WalkabilityMask | null,
): { x: number; y: number } {
  const base = Math.abs(hashStr(`floor1:${agentId}`));
  const zone = FLOOR1_AGENT_SPAWN_ZONES_SCENE[base % FLOOR1_AGENT_SPAWN_ZONES_SCENE.length];
  const step = 18;
  const cols = Math.max(1, Math.floor(zone.width / step));
  const rows = Math.max(1, Math.floor(zone.height / step));
  const total = cols * rows;
  let fallback: { x: number; y: number } | null = null;

  for (let offset = 0; offset < total; offset++) {
    const cell = (base + offset) % total;
    const col = cell % cols;
    const row = Math.floor(cell / cols);
    const sceneX = zone.x + Math.min(zone.width - 8, 10 + col * step);
    const sceneY = zone.y + Math.min(zone.height - 8, 10 + row * step);
    const point = sceneToWorldPoint(sceneX, sceneY, worldWidth, worldHeight);
    if (!fallback) fallback = point;
    if (!mask || isSafeWalkableMaskPoint(mask, sceneX, sceneY)) {
      return point;
    }
  }

  return fallback ?? sceneToWorldPoint(zone.x + zone.width / 2, zone.y + zone.height / 2, worldWidth, worldHeight);
}

function pickInitialPlayerSpawnWorld(worldWidth: number, worldHeight: number): { x: number; y: number } {
  return {
    x: (FLOOR1_SCENE_WIDTH / 2 / FLOOR1_SCENE_WIDTH) * worldWidth,
    y: (FLOOR1_SCENE_HEIGHT / 2 / FLOOR1_SCENE_HEIGHT) * worldHeight,
  };
}

// ---------------------------------------------------------------------------
// Office Layout Generator — BSP (Binary Space Partitioning) algorithm
// ---------------------------------------------------------------------------

/** Tile type identifiers used during layout generation.
 *  Must match the mapping in generateWorldGrid (pixel-tiles.ts). */
const LAYOUT_TILE = {
  VOID: 0,
  CARPET: 1,
  WOOD_FLOOR: 2,
  KITCHEN_TILE: 3,
  HALLWAY: 4,
  WALL: 5,
  GLASS_WALL: 6,
  DOOR: 7,
  WINDOW: 8,
  CONCRETE: 9,
  // Phase 1: Building shell tiles
  EXTERIOR_GROUND: 10,
  EXTERIOR_WALL: 11,
  WALL_TRIM: 12,
  FRONT_WALL_CUTAWAY: 13,
  WALL_BACK_TOP: 14,
} as const;

// Helper class for BSP layout generation
class BspNode {
  x: number;
  y: number;
  width: number;
  height: number;
  left: BspNode | null = null;
  right: BspNode | null = null;
  roomDef: any = null;
  roomRect: { x: number; y: number; width: number; height: number } | null = null;

  constructor(x: number, y: number, width: number, height: number) {
    this.x = x;
    this.y = y;
    this.width = width;
    this.height = height;
  }

  getLeaves(): BspNode[] {
    if (!this.left && !this.right) return [this];
    return [...(this.left?.getLeaves() || []), ...(this.right?.getLeaves() || [])];
  }
}

export function generateOfficeLayout(
  groups: { id: string; memberCount: number; name: string }[],
): OfficeLayout {
  // Phase 1 organic layout sizing
  const MIN_GROUP_COLUMNS = 3;
  const groupColumns = Math.max(MIN_GROUP_COLUMNS, Math.ceil(Math.sqrt(groups.length)));
  const numRows = Math.ceil(groups.length / groupColumns);

  // Island Pod dimensions
  const POD_W = 10;
  const POD_H = 8;
  const AISLE_W = 6;
  const PAD_OUTER = 8;

  const gridWidth = PAD_OUTER * 2 + groupColumns * POD_W + (groupColumns - 1) * AISLE_W;
  
  // Vertical layout: Top Service Area -> Group Pods -> Bottom Anchor (Reception)
  const topServiceRowY = PAD_OUTER;
  const groupsStartY = topServiceRowY + POD_H + AISLE_W;
  const bottomServiceRowY = groupsStartY + numRows * (POD_H + AISLE_W);
  const gridHeight = bottomServiceRowY + POD_H + PAD_OUTER;

  const grid = new Uint8Array(gridHeight * gridWidth);
  grid.fill(LAYOUT_TILE.WOOD_FLOOR); // Massive expansive main floor

  // Diorama Edge Logic: Void drop-off on South, Wall on North/West/East
  for (let y = 0; y < gridHeight; y++) {
    for (let x = 0; x < gridWidth; x++) {
      // Void boundary 
      if (y < PAD_OUTER - 2 || y > gridHeight - PAD_OUTER + 2 || x < PAD_OUTER - 2 || x > gridWidth - PAD_OUTER + 1) {
        grid[y * gridWidth + x] = LAYOUT_TILE.EXTERIOR_GROUND;
      }
      // Thick brick wall ONLY on North/West/East edges
      if (
        (y === PAD_OUTER - 2 && x >= PAD_OUTER - 2 && x <= gridWidth - PAD_OUTER + 1) ||
        (x === PAD_OUTER - 2 && y >= PAD_OUTER - 2 && y <= gridHeight - PAD_OUTER + 2) ||
        (x === gridWidth - PAD_OUTER + 1 && y >= PAD_OUTER - 2 && y <= gridHeight - PAD_OUTER + 2)
      ) {
        grid[y * gridWidth + x] = LAYOUT_TILE.EXTERIOR_WALL;
      }
    }
  }

  const rooms: OfficeRoom[] = [];

  const classifyRoomType = (
    group: { id: string; memberCount: number; name: string },
  ): OfficeRoom['roomType'] => {
    if (group.name.toLowerCase().includes('conference')) return 'conference';
    if (group.memberCount < 5) return 'lounge';
    if (group.memberCount <= 10) return 'workspace';
    return 'conference';
  };

  const createRoom = (
    id: string,
    memberCount: number,
    tileX: number,
    tileY: number,
    roomType: OfficeRoom['roomType'],
    width: number = POD_W,
    height: number = POD_H,
  ): OfficeRoom => ({
    id, memberCount, tileX, tileY, width, height, roomType,
    doorTileX: -1, doorTileY: -1, interactionPoints: [],
  });

  const carveRoom = (room: OfficeRoom) => {
    const isHardWalled = room.roomType === 'server' || room.roomType === 'manager';
    const floorTile =
      room.roomType === 'kitchen' || room.roomType === 'bathroom' ? LAYOUT_TILE.KITCHEN_TILE :
      room.roomType === 'lounge' || room.roomType === 'reception' || room.roomType === 'manager' ? LAYOUT_TILE.WOOD_FLOOR :
      room.roomType === 'server' ? LAYOUT_TILE.CONCRETE :
      LAYOUT_TILE.CARPET;

    for (let y = 0; y < room.height; y++) {
      for (let x = 0; x < room.width; x++) {
        const gx = room.tileX + x;
        const gy = room.tileY + y;
        
        if (isHardWalled && (x === 0 || x === room.width - 1 || y === 0 || y === room.height - 1)) {
          grid[gy * gridWidth + gx] = LAYOUT_TILE.WALL;
        } else {
          grid[gy * gridWidth + gx] = floorTile;
        }
      }
    }
    rooms.push(room);
  };

  const getSlotX = (col: number) => PAD_OUTER + col * (POD_W + AISLE_W);

  // Core Service Rooms Deployment
  carveRoom(createRoom('static_manager', 0, getSlotX(0), topServiceRowY, 'manager'));
  carveRoom(createRoom('static_kitchen', 0, getSlotX(groupColumns - 1), topServiceRowY, 'kitchen'));
  carveRoom(createRoom('static_server', 0, getSlotX(groupColumns - 1), bottomServiceRowY, 'server'));

  // Anchor Reception dynamically in bottom center
  const centerCol = Math.floor(groupColumns / 2);
  const recW = Math.max(POD_W, POD_W * 2);
  carveRoom(createRoom('static_reception', 0, getSlotX(centerCol), bottomServiceRowY, 'reception', Math.min(recW, gridWidth - PAD_OUTER*2), POD_H));

  // Scatter Pods for group chats
  groups.forEach((group, index) => {
    const r = Math.floor(index / groupColumns);
    const c = index % groupColumns;
    const room = createRoom(
      group.id,
      group.memberCount,
      getSlotX(c),
      groupsStartY + r * (POD_H + AISLE_W),
      classifyRoomType(group),
    );
    carveRoom(room);
  });

  // Only HardWalled rooms need doors now
  for (const room of rooms) {
    if (room.roomType === 'server' || room.roomType === 'manager') {
      room.doorTileX = room.tileX + Math.floor(room.width / 2);
      room.doorTileY = room.tileY + room.height - 1;
      grid[room.doorTileY * gridWidth + room.doorTileX] = LAYOUT_TILE.DOOR;
    }
    // No doors for open-plan islands!
  }

  // Windows along North Wall
  for (let c = PAD_OUTER; c < gridWidth - PAD_OUTER; c += 4) {
    if (grid[(PAD_OUTER - 2) * gridWidth + c] === LAYOUT_TILE.EXTERIOR_WALL) {
      grid[(PAD_OUTER - 2) * gridWidth + c] = LAYOUT_TILE.WINDOW;
    }
  }

  // Dollhouse top cuts 
  for (const room of rooms) {
    if (room.roomType === 'server' || room.roomType === 'manager') {
      const bottomY = room.tileY + room.height - 1;
      for (let x = 0; x < room.width; x++) {
        const gx = room.tileX + x;
        if (grid[bottomY * gridWidth + gx] === LAYOUT_TILE.WALL) {
           grid[bottomY * gridWidth + gx] = LAYOUT_TILE.FRONT_WALL_CUTAWAY;
        }
      }
      const aboveY = room.tileY - 1;
      for (let x = 0; x < room.width; x++) {
        const gx = room.tileX + x;
        grid[aboveY * gridWidth + gx] = LAYOUT_TILE.WALL_BACK_TOP;
      }
    }
  }

  // Interaction Points mapping exactly to pixel-houses.ts bounds
  // Keeping safe offsets from edges so furniture doesn't clip
  for (const room of rooms) {
    const innerX = room.tileX + 1;
    const innerY = room.tileY + 1;
    const innerWidth = room.width - 2;
    const innerHeight = room.height - 2;

    switch (room.roomType) {
      case 'workspace':
        for (let y = 0; y < Math.floor(innerHeight / 3); y++) {
          for (let x = 0; x < Math.floor(innerWidth / 3); x++) {
            room.interactionPoints.push({
              tileX: innerX + x * 3 + 1,
              tileY: innerY + y * 3 + 1,
              type: 'desk',
            });
          }
        }
        break;
      case 'conference': {
        const tableWidth = Math.max(1, innerWidth - 4);
        const tableHeight = Math.max(1, innerHeight - 4);
        for (let y = 0; y < tableHeight; y++) {
          for (let x = 0; x < tableWidth; x++) {
            room.interactionPoints.push({ tileX: innerX + 2 + x, tileY: innerY + 2 + y, type: 'desk' });
          }
        }
        room.interactionPoints.push({ tileX: innerX + Math.floor(innerWidth / 2), tileY: innerY, type: 'whiteboard' });
        break;
      }
      case 'lounge':
        room.interactionPoints.push({ tileX: innerX + 1, tileY: innerY + 1, type: 'sofa' });
        room.interactionPoints.push({ tileX: innerX + 2, tileY: innerY + 1, type: 'sofa' });
        break;
      case 'server':
        for (let i = 0; i < innerHeight - 1; i++) {
          room.interactionPoints.push({ tileX: innerX + 1, tileY: innerY + i, type: 'server_rack' });
        }
        break;
      case 'kitchen':
        for (let i = 0; i < innerWidth - 1; i++) {
          room.interactionPoints.push({ tileX: innerX + i, tileY: innerY, type: 'kitchen_counter' });
        }
        break;
      case 'reception':
        for (let i = 0; i < Math.min(4, innerWidth); i++) {
          room.interactionPoints.push({ tileX: room.tileX + Math.floor(room.width / 2) - 2 + i, tileY: room.tileY + 2, type: 'reception_desk' });
        }
        break;
      case 'manager':
        room.interactionPoints.push({ tileX: innerX + Math.floor(innerWidth / 2), tileY: innerY + 1, type: 'desk' });
        room.interactionPoints.push({ tileX: innerX + 1, tileY: innerY + Math.floor(innerHeight / 2) + 1, type: 'desk' });
        room.interactionPoints.push({ tileX: innerX + innerWidth - 2, tileY: innerY + Math.floor(innerHeight / 2) + 1, type: 'desk' });
        break;
      case 'bathroom':
        room.interactionPoints.push({ tileX: innerX + 1, tileY: innerY, type: 'sink' });
        room.interactionPoints.push({ tileX: innerX + 2, tileY: innerY, type: 'sink' });
        break;
    }
  }

  return { rooms, grid, gridWidth, gridHeight };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * BFS pathfinder over a PNG walkability mask (white pixels = walkable).
 *
 * Sampling: the mask is sub-divided into MASK_PATH_TILE-sized cells; a cell
 * is walkable iff its center pixel passes the white test. Inputs/outputs are
 * in WORLD coordinates — conversion to mask pixel space is handled here
 * using `worldWidth/worldHeight` (the store's logical extent).
 *
 * Hard rule: if no path is found, returns `null`. Callers MUST treat null
 * as "don't walk" rather than falling back to a straight line — that was
 * the source of through-wall behavior in the BSP-grid path.
 *
 * Robustness against off-by-one start/dest tiles:
 *  - If the start cell is a wall (e.g. agent.anim.x rounds onto a 1px wall
 *    edge), we BFS outward up to MASK_SNAP_RADIUS to find the nearest
 *    walkable tile and start from there.
 *  - Same for the destination.
 */
const MASK_PATH_TILE = 16;       // mask sample step (px in scene/mask space)
const MASK_SNAP_RADIUS = 6;      // tiles to search when snapping start/dest
const MASK_MAX_ITERATIONS = 8000;

function findPathOnMask(
  mask: WalkabilityMask,
  worldWidth: number,
  worldHeight: number,
  startWorldX: number,
  startWorldY: number,
  destWorldX: number,
  destWorldY: number,
): { x: number; y: number }[] | null {
  if (worldWidth <= 0 || worldHeight <= 0) return null;

  const wToMx = mask.width / worldWidth;
  const wToMy = mask.height / worldHeight;
  const mToWx = worldWidth / mask.width;
  const mToWy = worldHeight / mask.height;

  const cols = Math.max(1, Math.floor(mask.width / MASK_PATH_TILE));
  const rows = Math.max(1, Math.floor(mask.height / MASK_PATH_TILE));

  const isWalkableTile = (col: number, row: number): boolean => {
    if (col < 0 || row < 0 || col >= cols || row >= rows) return false;
    const px = col * MASK_PATH_TILE + MASK_PATH_TILE / 2;
    const py = row * MASK_PATH_TILE + MASK_PATH_TILE / 2;
    const idx = (Math.floor(py) * mask.width + Math.floor(px)) * 4;
    const a = mask.pixels[idx + 3];
    if (a < 16) return false;
    const r = mask.pixels[idx];
    const g = mask.pixels[idx + 1];
    const b = mask.pixels[idx + 2];
    return r > 240 && g > 240 && b > 240;
  };

  const idxOf = (c: number, r: number) => r * cols + c;

  const snapToWalkable = (col: number, row: number): { c: number; r: number } | null => {
    if (isWalkableTile(col, row)) return { c: col, r: row };
    const seen = new Set<number>();
    const q: Array<{ c: number; r: number; d: number }> = [{ c: col, r: row, d: 0 }];
    seen.add(idxOf(col, row));
    while (q.length) {
      const { c, r, d } = q.shift()!;
      if (d >= MASK_SNAP_RADIUS) continue;
      for (const [dc, dr] of [[0, -1], [0, 1], [-1, 0], [1, 0]] as const) {
        const nc = c + dc;
        const nr = r + dr;
        if (nc < 0 || nr < 0 || nc >= cols || nr >= rows) continue;
        const ni = idxOf(nc, nr);
        if (seen.has(ni)) continue;
        seen.add(ni);
        if (isWalkableTile(nc, nr)) return { c: nc, r: nr };
        q.push({ c: nc, r: nr, d: d + 1 });
      }
    }
    return null;
  };

  const startCol = Math.floor(startWorldX * wToMx / MASK_PATH_TILE);
  const startRow = Math.floor(startWorldY * wToMy / MASK_PATH_TILE);
  const destCol = Math.floor(destWorldX * wToMx / MASK_PATH_TILE);
  const destRow = Math.floor(destWorldY * wToMy / MASK_PATH_TILE);

  const start = snapToWalkable(startCol, startRow);
  const dest = snapToWalkable(destCol, destRow);
  if (!start || !dest) return null;

  if (start.c === dest.c && start.r === dest.r) {
    return [{ x: destWorldX, y: destWorldY }];
  }

  const queue: Array<{ c: number; r: number; path: Array<{ c: number; r: number }> }> = [
    { c: start.c, r: start.r, path: [] },
  ];
  const visited = new Set<number>();
  visited.add(idxOf(start.c, start.r));

  let bestPath: Array<{ c: number; r: number }> | null = null;
  let iterations = 0;
  while (queue.length && iterations < MASK_MAX_ITERATIONS) {
    iterations++;
    const { c, r, path } = queue.shift()!;
    if (c === dest.c && r === dest.r) {
      bestPath = path;
      break;
    }
    for (const [dc, dr] of [[0, -1], [0, 1], [-1, 0], [1, 0]] as const) {
      const nc = c + dc;
      const nr = r + dr;
      if (!isWalkableTile(nc, nr)) continue;
      const ni = idxOf(nc, nr);
      if (visited.has(ni)) continue;
      visited.add(ni);
      queue.push({ c: nc, r: nr, path: [...path, { c: nc, r: nr }] });
    }
  }

  if (!bestPath) return null;

  return bestPath.map((p) => ({
    x: (p.c * MASK_PATH_TILE + MASK_PATH_TILE / 2) * mToWx,
    y: (p.r * MASK_PATH_TILE + MASK_PATH_TILE / 2) * mToWy,
  }));
}

/**
 * A* Pathfinding (BFS variant for small 16x16 grid sizes).
 * Yields discrete pixel waypoints to navigate around walls.
 */
export function findPath(
  grid: TileGrid,
  startX: number,
  startY: number,
  destX: number,
  destY: number,
): { x: number; y: number }[] {
  const startCol = Math.floor(startX / 16);
  const startRow = Math.floor(startY / 16);
  const destCol = Math.floor(destX / 16);
  const destRow = Math.floor(destY / 16);

  if (startCol === destCol && startRow === destRow) {
    return [{ x: destX, y: destY }];
  }

  const isWalkable = (col: number, row: number) => {
    if (col < 0 || col >= grid.cols || row < 0 || row >= grid.rows) return false;
    const tile = grid.layers[0][row * grid.cols + col];
    // 1=CARPET, 2=WOOD_FLOOR, 3=KITCHEN_TILE, 4=HALLWAY, 7=DOOR, 9=CONCRETE
    return tile === 1 || tile === 2 || tile === 3 || tile === 4 || tile === 7 || tile === 9;
  };

  const toIndex = (c: number, r: number) => r * grid.cols + c;
  
  const queue = [{ c: startCol, r: startRow, path: [] as {c:number, r:number}[] }];
  const visited = new Set<number>();
  visited.add(toIndex(startCol, startRow));

  // prioritize moving horizontally and vertically
  const dirs = [
    { dc: 0, dr: -1 }, { dc: 0, dr: 1 }, { dc: -1, dr: 0 }, { dc: 1, dr: 0 },
  ];

  let bestPath: {c:number, r:number}[] | null = null;
  let iterations = 0;

  while (queue.length > 0 && iterations < 3000) {
    iterations++;
    const { c, r, path } = queue.shift()!;

    if (c === destCol && r === destRow) {
      bestPath = path;
      break;
    }

    for (const d of dirs) {
      const nc = c + d.dc;
      const nr = r + d.dr;
      const idx = toIndex(nc, nr);

      if (isWalkable(nc, nr) && !visited.has(idx)) {
        visited.add(idx);
        queue.push({ c: nc, r: nr, path: [...path, { c: nc, r: nr }] });
      }
    }
  }

  if (bestPath && bestPath.length > 0) {
    return bestPath.map(p => ({ x: p.c * 16 + 8, y: p.r * 16 + 8 }));
  }

  return [{ x: destX, y: destY }];
}

export function findMostRecentHouse(
  agentId: string,
  messagesByConv: Record<string, any[]>,
  houseIds: Set<string>,
): string | null {
  let latest: { convId: string; time: number } | null = null;

  for (const convId of houseIds) {
    const msgs = messagesByConv[convId];
    if (!msgs) continue;
    for (let i = msgs.length - 1; i >= 0; i--) {
      const msg = msgs[i];
      const senderId = msg.sender_id ?? msg.senderId ?? msg.user_id ?? msg.userId;
      if (senderId === agentId) {
        const ts =
          typeof msg.created_at === 'string'
            ? new Date(msg.created_at).getTime()
            : typeof msg.timestamp === 'number'
              ? msg.timestamp
              : 0;
        if (!latest || ts > latest.time) {
          latest = { convId, time: ts };
        }
        break;
      }
    }
  }

  return latest?.convId ?? null;
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

export const usePixelWorldStore = create<PixelWorldState>()((set, get) => ({
  agents: new Map(),
  houses: new Map(),
  worldWidth: 0,
  worldHeight: 0,
  cameraX: 0,
  cameraY: 0,
  targetCameraX: 0,
  targetCameraY: 0,
  // Default camera zoom. At TILE_SIZE=16 this gives 48 rendered px per
  // tile — closer to the Stardew-style "1 tile = 64 px" feel than the old
  // zoom=2 (32 px/tile) which made characters look half their intended
  // size. Keeping `zoom` and `targetZoom` in sync so the initial render
  // doesn't lerp from a jarring wrong value.
  zoom: 3,
  targetZoom: 3,
  player: null as PlayerState | null,
  particles: [] as Particle[],
  timeOfDay: 0.25,
  isOpen: false,
  tileGrid: null as TileGrid | null,
  walkabilityMask: null as WalkabilityMask | null,
  chunkCache: null as ChunkCache | null,
  frameCount: 0,
  instanceId: null as string | null,

  // ─── movePlayer ─────────────────────────────────────────────────────────
  movePlayer: (dx: number, dy: number) => {
    (globalThis as any).__choruz_player_moved = true;
    set((state) => {
      if (!state.player) return state;
      const speed = 6;
      let newX = state.player.x + dx * speed;
      let newY = state.player.y + dy * speed;

      // ---- Collision Detection ----
      if (state.tileGrid) {
        const grid = state.tileGrid;
        const checkCollision = (cx: number, cy: number) => {
          // 12x8 bounding box around player foot origin
          const l = Math.floor((cx - 6) / 16);
          const r = Math.floor((cx + 6) / 16);
          const t = Math.floor((cy - 4) / 16);
          const b = Math.floor((cy + 2) / 16);

          const isWall = (col: number, row: number) => {
            if (col < 0 || col >= grid.cols || row < 0 || row >= grid.rows) return true;
            const tile = grid.layers[0][row * grid.cols + col];
            // Fast check for solid tiles based on pixel-tiles.ts Tile enum map
            return tile === 6 || tile === 7 || tile === 9 || tile === 10 || tile === 30 || (tile >= 11 && tile <= 26);
          };

          return isWall(l, t) || isWall(r, t) || isWall(l, b) || isWall(r, b);
        };

        if (dx !== 0 && checkCollision(newX, state.player.y)) {
          newX = state.player.x; // Block X sliding
        }
        if (dy !== 0 && checkCollision(newX, newY)) {
          newY = state.player.y; // Block Y sliding
        }
      }

      newX = Math.max(0, Math.min(state.worldWidth - 48, newX));
      newY = Math.max(0, Math.min(state.worldHeight - 48, newY));

      let dir = state.player.direction;
      if (Math.abs(dx) > Math.abs(dy)) {
        dir = dx > 0 ? 'right' : 'left';
      } else if (Math.abs(dy) > 0) {
        dir = dy > 0 ? 'down' : 'up';
      }

      // Sync to the dummy 'player' agent so the renderer draws it depth-sorted
      const agents = new Map(state.agents);
      let playerAgent = agents.get('player');
      if (playerAgent) {
        playerAgent = {
          ...playerAgent,
          anim: {
            ...playerAgent.anim,
            x: newX,
            y: newY,
            state: 'walk' as const,
            direction: dir,
          },
        };
        agents.set('player', playerAgent);
      }

      return {
        player: { ...state.player, x: newX, y: newY, direction: dir },
        agents,
      };
    });
    get().updatePlayerRoom();
  },

  // ─── updatePlayerRoom ──────────────────────────────────────────────────
  updatePlayerRoom: () => {
    set((state) => {
      if (!state.player) return state;
      let foundRoomId: string | null = null;

      for (const house of state.houses.values()) {
        const hx = house.worldX;
        const hy = house.worldY;
        const hw = house.room.width * 16;
        const hh = house.room.height * 16;
        if (
          state.player.x >= hx && state.player.x <= hx + hw &&
          state.player.y >= hy && state.player.y <= hy + hh
        ) {
          foundRoomId = house.id;
          break;
        }
      }

      if (state.player.currentRoomId !== foundRoomId) {
        const targetZoom = foundRoomId ? 3 : 2;
        const agents = new Map(state.agents);
        const playerAgent = agents.get('player');
        if (playerAgent) {
          agents.set('player', { ...playerAgent, currentHouseId: foundRoomId });
        }
        return {
          player: { ...state.player, currentRoomId: foundRoomId },
          targetZoom,
          agents,
        };
      }
      return state;
    });
  },

  // ─── popBubble ──────────────────────────────────────────────────────────
  popBubble: (agentId: string, emoji: string, ticks = 50) => {
    set((state) => {
      const agents = new Map(state.agents);
      const agent = agents.get(agentId);
      if (agent) {
        agents.set(agentId, {
          ...agent,
          anim: {
            ...agent.anim,
            bubble: emoji,
            bubbleTicks: ticks,
          },
        });
        return { agents };
      }
      return state;
    });
  },

  // ─── initialize ─────────────────────────────────────────────────────────
  initialize(conversations, agentsList, messagesByConv) {
    // Filter to group conversations
    const groups = conversations.filter(
      (c: any) =>
        c.conversation_type === 'group' || c.type === 'group',
    );

    // Build group descriptors for the layout generator
    const groupDefs = groups.map((conv: any, idx: number) => {
      const convId: string = conv.id ?? conv.conversation_id;
      const members: string[] = Array.isArray(conv.members)
        ? conv.members
        : typeof conv.members === 'object' && conv.members
          ? Object.keys(conv.members)
          : [];
      return {
        id: convId,
        memberCount: members.length || 1,
        name: conv.name ?? `Group ${idx + 1}`,
        members,
      };
    });

    // Generate office floor plan
    const layout = generateOfficeLayout(groupDefs);

    // Invalidate cached lightmap since layout changed (Fix 4)
    invalidateLightmap();

    // Clear cached room canvases for fresh layout
    clearRoomCache();

    // Build house info from layout rooms
    const houses = new Map<string, HouseInfo>();

    for (const room of layout.rooms) {
      // Skip static rooms for house lookup (they don't map to conversations)
      const groupDef = groupDefs.find(g => g.id === room.id);
      if (!groupDef) continue;

      const worldX = room.tileX * TILE_SIZE;
      const worldY = room.tileY * TILE_SIZE;

      houses.set(room.id, {
        id: room.id,
        name: groupDef.name,
        house: generateHouse(room.id, groupDef.memberCount, room),
        worldX,
        worldY,
        members: groupDef.members,
        room,
      });
    }

    // Compute world dimensions from grid
    const worldWidth = layout.gridWidth * TILE_SIZE;
    const worldHeight = layout.gridHeight * TILE_SIZE;

    // Build agent states — incremental: reuse existing sprites (Fix 6)
    const houseIds = new Set(houses.keys());
    const agents = new Map<string, PixelAgentState>();
    const prevAgents = get().agents;
    const newAgentIds = new Set<string>();

    for (const agent of agentsList) {
      const id: string = agent.id ?? agent.agent_id;
      const name: string = agent.name ?? agent.display_name ?? id;
      newAgentIds.add(id);

      // Reuse existing sprite if agent already existed (avoid re-generation)
      const existing = prevAgents.get(id);
      const sprite = existing ? existing.sprite : generateSprite(id);
      const initialSpawn = pickInitialAgentSpawnWorld(id, worldWidth, worldHeight, get().walkabilityMask);

      agents.set(id, {
        id,
        name,
        sprite,
        anim: existing ? existing.anim : createAgentAnim(initialSpawn.x, initialSpawn.y),
        currentHouseId: existing ? existing.currentHouseId : null,
        homeX: existing ? existing.homeX : initialSpawn.x,
        homeY: existing ? existing.homeY : initialSpawn.y,
      });
    }

    // Dispose sprites for agents that no longer exist (immediate memory release)
    for (const [oldId, oldAgent] of prevAgents) {
      if (oldId !== 'player' && !newAgentIds.has(oldId)) {
        // Force release of generated sprite frame canvases
        const frames = oldAgent.sprite.frames;
        for (const key of Object.keys(frames) as (keyof typeof frames)[]) {
          const strip = frames[key];
          if (Array.isArray(strip)) {
            for (const frame of strip) {
              if (frame && 'width' in frame) {
                (frame as any).width = 0;
                (frame as any).height = 0;
              }
            }
          }
        }
      }
    }

    // Generate tile grid from the layout
    const tileGrid = generateWorldGrid(layout);
    const chunkCache = createChunkCache();

    const initialPlayerSpawn = pickInitialPlayerSpawnWorld(worldWidth, worldHeight);
    const startPlayerX = initialPlayerSpawn.x;
    const startPlayerY = initialPlayerSpawn.y;

    const playerSprite = generateSprite('player_avatar');
    playerSprite.palette = ['#FFD700', '#FFA500', '#DAA520', '#B8860B'];

    const playerAgent: PixelAgentState = {
      id: 'player',
      name: 'You',
      sprite: playerSprite,
      anim: createAgentAnim(startPlayerX, startPlayerY),
      currentHouseId: null,
      homeX: startPlayerX,
      homeY: startPlayerY,
    };
    agents.set('player', playerAgent);

    set({
      agents,
      houses,
      worldWidth,
      worldHeight,
      tileGrid,
      chunkCache,
      player: {
        x: startPlayerX,
        y: startPlayerY,
        direction: 'down' as const,
        currentRoomId: null,
        sprite: playerSprite,
      },
      // Match the initial-state default (see near top of this file).
      // `initialize()` is called on every data refresh; if we leave this at
      // 2 the user's zoom resets every time conversations change.
      targetZoom: 3,
      zoom: 3,
      // Fresh id per init — stable across the remaining lifetime of this store
      // instance so every event in a single session can be correlated, but
      // regenerated on remount so a reload doesn't look like the old session.
      instanceId: newPixelInstanceId(),
    });

    // ── Load PNG sprite assets (non-blocking, progressive enhancement) ──
    // Only load once — subsequent initialize() calls skip this.
    if (!(globalThis as any).__choruz_png_loaded) {
      (globalThis as any).__choruz_png_loading = true;
      loadTileSheets().then(() => {
        (globalThis as any).__choruz_png_loaded = true;
        const currentState = get();
        if (currentState.tileGrid) {
          const freshCache = createChunkCache();
          set({ chunkCache: freshCache });
        }
      }).catch(() => {});
    }
  },

  // ─── handleMessage ──────────────────────────────────────────────────────
  // Agent received/sent a message → walk to the player's current position
  // (snapshot). Path is computed on the floor's PNG walkability mask so the
  // agent respects the same visible walls the player does. Player movement
  // *after* the snapshot does NOT pull the agent — they commit to a fixed
  // target and stop on arrival.
  handleMessage(senderId, conversationId) {
    const state0 = get();
    const { agents, instanceId, player } = state0;
    const agent = agents.get(senderId);
    if (!agent) return;
    emitPix(instanceId, 'pixel_agent_message', {
      agent_id: senderId,
      agent_name: agent.name,
      conversation_id: conversationId,
      from_state: agent.anim.state,
      x: agent.anim.x,
      y: agent.anim.y,
      ticks_in_state: agent.anim.ticksInState,
      current_conversation_id: agent.currentHouseId,
    });

    if (!player) return; // No player yet — nothing to walk toward.

    // Snapshot player position. This is FROZEN for the rest of this walk:
    // even if the player wanders away, the agent finishes at the snapshot.
    const targetX = player.x;
    const targetY = player.y;

    // Stopping next to the player puts the agent in `idle` (no desk/sofa to
    // sit at — they're standing in the open, near the player). The talking
    // gesture is a short flash applied either right now (if already nearby)
    // or by the post-arrival decay otherwise.
    const arrivalState: AnimState = 'idle';

    const distToPlayer = Math.hypot(targetX - agent.anim.x, targetY - agent.anim.y);
    const ALREADY_NEAR_PLAYER_PX = 32;
    const alreadyNear = distToPlayer <= ALREADY_NEAR_PLAYER_PX && agent.anim.state !== 'walk';

    let newAnim: AgentAnim;
    if (alreadyNear) {
      newAnim = startTalking(agent.anim);
    } else {
      const state = get();
      let path: { x: number; y: number }[] | null = null;
      if (state.walkabilityMask) {
        path = findPathOnMask(
          state.walkabilityMask,
          state.worldWidth,
          state.worldHeight,
          agent.anim.x,
          agent.anim.y,
          targetX,
          targetY,
        );
      }

      if (!path) {
        // No reachable path (mask not loaded yet, or player is in an
        // isolated region). Refuse to teleport through walls — surface the
        // message via a bubble and leave the agent in place.
        newAnim = { ...agent.anim, bubble: '💬', bubbleTicks: 30 };
      } else {
        const isFar = distToPlayer > 16;
        const bubble = isFar ? '🏃' : '💭';
        const walkedAnim = startWalking(
          agent.anim,
          targetX,
          targetY,
          conversationId,
          path,
          arrivalState,
        );
        newAnim = { ...walkedAnim, bubble, bubbleTicks: 60 };
        emitPix(instanceId, 'pixel_agent_walk_started', {
          agent_id: senderId,
          conversation_id: conversationId,
          from_state: agent.anim.state,
          arrival_state: arrivalState,
          point_type: 'player',
          x: agent.anim.x,
          y: agent.anim.y,
          target_x: targetX,
          target_y: targetY,
          reason: 'handle_message_to_player',
        });
      }
    }

    const next = new Map(agents);
    next.set(senderId, {
      ...agent,
      anim: newAnim,
      currentHouseId: conversationId,
    });
    set({ agents: next });
  },

  // ─── handleMention ──────────────────────────────────────────────────────
  handleMention(agentId) {
    const { agents, instanceId } = get();
    const agent = agents.get(agentId);
    if (!agent) return;

    // Don't blow away an in-flight walk: if the agent is currently walking,
    // keep them walking and surface the mention only as the overhead bubble.
    // Overwriting state to 'thinking' here would cancel the walk and strand
    // them in the hallway with no way to reach the conversation room.
    const isWalking = agent.anim.state === 'walk';
    const fromState = agent.anim.state;
    const nextAnim = isWalking
      ? { ...agent.anim, bubble: '💡', bubbleTicks: 80 }
      : { ...startThinking(agent.anim), bubble: '💡', bubbleTicks: 80 };

    emitPix(instanceId, 'pixel_agent_mention', {
      agent_id: agentId,
      agent_name: agent.name,
      from_state: fromState,
      to_state: nextAnim.state,
      walk_preserved: isWalking,
      x: agent.anim.x,
      y: agent.anim.y,
      ticks_in_state: agent.anim.ticksInState,
      // Conversation the agent is physically inside right now (null in
      // hallway / common area). Named `current_conversation_id` so it
      // doesn't collide with `conversation_id` (the event's target).
      current_conversation_id: agent.currentHouseId,
      resume_state: nextAnim.resumeState ?? null,
    });
    if (!isWalking) {
      emitPix(instanceId, 'pixel_agent_thinking_started', {
        agent_id: agentId,
        from_state: fromState,
        resume_state: nextAnim.resumeState ?? null,
      });
    }

    const next = new Map(agents);
    next.set(agentId, { ...agent, anim: nextAnim });

    set({ agents: next });
  },

  // ─── wanderAgentInRoom ──────────────────────────────────────────────────
  // Ambient idle pacing. The legacy version walked agents to BSP-generated
  // interactionPoints (phantom desks/sofas not visible on the floor PNG) —
  // that's what produced the "agent walks through a wall to nothing" bug
  // after a handleMessage walk completed. New behavior: pick a random
  // target within a small radius of the agent's *current* position and
  // pathfind on the same PNG mask the player uses. If the mask isn't
  // loaded yet, or no walkable target is found in a few tries, we no-op.
  wanderAgentInRoom(agentId) {
    const { agents, walkabilityMask, worldWidth, worldHeight, instanceId } = get();
    const agent = agents.get(agentId);
    if (!agent) return;
    if (agent.anim.state !== 'idle') return;
    if (!walkabilityMask) return;

    // Wander center is the agent's *home* (spawn anchor), not the current
    // position — otherwise repeated wanders random-walk the agent across
    // the floor and they never return after a handleMessage trip to the
    // player. With this anchor, after replying the agent naturally drifts
    // back toward home over the next few wander ticks.
    const WANDER_RADIUS_PX = 80;
    const MIN_RADIUS_PX = 32;
    let path: { x: number; y: number }[] | null = null;
    let targetX = agent.homeX;
    let targetY = agent.homeY;
    for (let attempt = 0; attempt < 12; attempt++) {
      const angle = Math.random() * Math.PI * 2;
      const dist = MIN_RADIUS_PX + Math.random() * (WANDER_RADIUS_PX - MIN_RADIUS_PX);
      targetX = agent.homeX + Math.cos(angle) * dist;
      targetY = agent.homeY + Math.sin(angle) * dist;

      const sceneX = (targetX / worldWidth) * walkabilityMask.width;
      const sceneY = (targetY / worldHeight) * walkabilityMask.height;
      if (!isSafeWalkableMaskPoint(walkabilityMask, sceneX, sceneY)) continue;

      path = findPathOnMask(
        walkabilityMask,
        worldWidth,
        worldHeight,
        agent.anim.x,
        agent.anim.y,
        targetX,
        targetY,
      );
      if (path && path.length > 0) break;
    }
    if (!path) return;

    const newAnim = startWalking(
      agent.anim,
      targetX,
      targetY,
      agent.currentHouseId,
      path,
      'idle',
    );
    const next = new Map(agents);
    next.set(agentId, { ...agent, anim: newAnim });
    set({ agents: next });
    emitPix(instanceId, 'pixel_agent_wander_started', {
      agent_id: agentId,
      conversation_id: agent.currentHouseId,
      point_type: 'mask_radius',
      x: agent.anim.x,
      y: agent.anim.y,
      target_x: targetX,
      target_y: targetY,
    });
  },

  // ─── tick ───────────────────────────────────────────────────────────────
  tick(displayWidth?: number, displayHeight?: number) {
    const state = get();
    const { agents, cameraX, cameraY, targetCameraX, targetCameraY, particles, timeOfDay, frameCount, houses, player, targetZoom, zoom } = state;

    const newFrameCount = frameCount + 1;
    // Full day cycle = 18000 frames (~5 min at 60fps). Was 1800 (30 sec) — way too fast.
    const newTimeOfDay = (timeOfDay + 1 / 18000) % 1;
    let changed = false;
    const next = new Map<string, PixelAgentState>();
    const newParticles: Particle[] = [];

    for (const [id, agent] of agents) {
      const prevAnim = agent.anim;
      let newAnim = tickAgent(prevAnim);

      // ── State transition telemetry (observability for debugging stuck agents) ──
      // Emitted here rather than in tickAgent because tickAgent is pure and
      // must stay side-effect free (the pure-function E2E relies on that).
      if (prevAnim.state === 'walk' && newAnim.state !== 'walk') {
        emitPix(state.instanceId, 'pixel_agent_walk_completed', {
          agent_id: id,
          from_state: 'walk',
          to_state: newAnim.state,
          x: newAnim.x,
          y: newAnim.y,
          target_x: prevAnim.targetX,
          target_y: prevAnim.targetY,
          ticks_in_state: prevAnim.ticksInState,
        });
      }
      if (prevAnim.state === 'talking' && newAnim.state === 'typing') {
        emitPix(state.instanceId, 'pixel_agent_talking_decayed', {
          agent_id: id,
          from_state: 'talking',
          to_state: 'typing',
          ticks_in_state: prevAnim.ticksInState,
        });
      }

      // Decay: agents in a short-lived attention state for more than 60 ticks
      // leave that state. Covers legacy 'work'/'think' plus the new status
      // 'thinking'. If the agent stashed a `resumeState` (set by
      // `startThinking` when entering from typing/sitting), restore to that
      // pose so a seated agent doesn't wander off after a mention. Otherwise
      // fall back to 'idle'. Persistent desk states (typing/sitting) and
      // 'talking' (which decays to 'typing' via tickAgent) are excluded.
      if (
        (newAnim.state === 'work' ||
          newAnim.state === 'think' ||
          newAnim.state === 'thinking') &&
        newAnim.ticksInState > 60
      ) {
        const restore: AnimState = newAnim.resumeState ?? 'idle';
        const fromState = newAnim.state;
        const resumedFrom = newAnim.resumeState;
        newAnim = {
          ...newAnim,
          state: restore,
          frame: 0,
          ticksInState: 0,
          resumeState: undefined,
        };
        if (fromState === 'thinking') {
          emitPix(state.instanceId, 'pixel_agent_thinking_resolved', {
            agent_id: id,
            from_state: 'thinking',
            to_state: restore,
            resume_state: resumedFrom ?? null,
          });
        }
      }

      // ── Walking dust particles (all agents are human) ──
      if (newAnim.state === 'walk' && newAnim.ticksInState % 4 === 0) {
        const pColor = DUST_COLORS[Math.floor(Math.random() * DUST_COLORS.length)];
        const baseX = newAnim.x + 8 + (Math.random() - 0.5) * 6;
        const baseY = newAnim.y + 28 + Math.random() * 4;

        newParticles.push({
          x: baseX, y: baseY,
          vx: (Math.random() - 0.5) * 0.8,
          vy: -0.3 - Math.random() * 0.4,
          life: 20 + Math.floor(Math.random() * 10),
          maxLife: 30,
          size: 1 + Math.random() * 1.5,
          color: pColor,
          type: 'dust',
        });
      }

      if (newAnim !== agent.anim) {
        changed = true;
        next.set(id, { ...agent, anim: newAnim });
      } else {
        next.set(id, { ...agent });
      }
    }

    // ── Ambient particles: dust motes, steam from mugs, water cooler bubbles ──
    // Only spawn ambient particles at reasonable intervals to keep count manageable.

    // Dust motes in sunlight — maintain ~25 floating particles near windows
    // We spawn a few each tick to replace fading ones. Windows are on exterior walls.
    if (newFrameCount % 3 === 0) {
      // Count existing dust motes in current particles
      let existingDustMotes = 0;
      for (const p of particles) {
        if (p.type === 'dust_mote') existingDustMotes++;
      }
      for (const np of newParticles) {
        if (np.type === 'dust_mote') existingDustMotes++;
      }
      if (existingDustMotes < 25) {
        // Spawn near a random house (simulating sunlight through windows)
        const houseArr = Array.from(houses.values());
        if (houseArr.length > 0) {
          const h = houseArr[newFrameCount % houseArr.length];
          const roomCenterX = h.worldX + (h.room.width * TILE_SIZE) / 2;
          const roomCenterY = h.worldY + (h.room.height * TILE_SIZE) / 2;
          // Spawn within a sunlight shaft area near the room
          const spawnX = roomCenterX + (Math.random() - 0.5) * h.room.width * TILE_SIZE * 0.6;
          const spawnY = roomCenterY + (Math.random() - 0.5) * h.room.height * TILE_SIZE * 0.4;
          newParticles.push({
            x: spawnX,
            y: spawnY,
            vx: (Math.random() - 0.5) * 0.15,
            vy: -0.05 + (Math.random() - 0.5) * 0.1,
            life: 90 + Math.floor(Math.random() * 60),
            maxLife: 150,
            size: 1 + Math.random(),
            color: '#FFEEB3',
            type: 'dust_mote',
            alpha: 0.3 + Math.random() * 0.5,
          });
        }
      }
    }

    // Steam from coffee mugs — pick one random eligible room per spawn tick
    if (newFrameCount % 45 === 0) {
      const eligibleHouses = Array.from(houses.values()).filter(h =>
        h.room.roomType === 'kitchen' || h.room.roomType === 'workspace' || h.room.roomType === 'conference'
      );
      if (eligibleHouses.length > 0) {
        const h = eligibleHouses[Math.floor(Math.random() * eligibleHouses.length)];
        // Mug position approximation: near a desk/counter
        const mugX = h.worldX + (h.room.width * TILE_SIZE) * 0.4 + Math.random() * TILE_SIZE * 2;
        const mugY = h.worldY + TILE_SIZE * 2 + Math.random() * TILE_SIZE;
        newParticles.push({
          x: mugX,
          y: mugY,
          vx: 0,
          vy: -0.4 - Math.random() * 0.2,
          life: 40 + Math.floor(Math.random() * 20),
          maxLife: 60,
          size: 1.5,
          color: '#FFFFFF',
          type: 'steam',
          alpha: 0.35,
        });
      }
    }

    // Water cooler bubbles — pick one random eligible room per spawn tick
    if (newFrameCount % 120 === 0) {
      const eligibleHouses = Array.from(houses.values()).filter(h =>
        h.room.roomType === 'kitchen' && h.room.width >= 6
      );
      if (eligibleHouses.length > 0) {
        const h = eligibleHouses[Math.floor(Math.random() * eligibleHouses.length)];
        // Water cooler is near the right side of kitchen
        const coolerX = h.worldX + (h.room.width - 2) * TILE_SIZE + TILE_SIZE * 0.5;
        const coolerY = h.worldY + TILE_SIZE * 1.8;
        newParticles.push({
          x: coolerX,
          y: coolerY,
          vx: 0,
          vy: -0.8,
          life: 15 + Math.floor(Math.random() * 5),
          maxLife: 20,
          size: 2,
          color: '#AADDFF',
          type: 'water_bubble',
          alpha: 0.6,
        });
      }
    }

    // Update existing particles
    const updatedParticles: Particle[] = [];
    for (const p of particles) {
      const np = { ...p, x: p.x + p.vx, y: p.y + p.vy, life: p.life - 1 };
      switch (p.type) {
        case 'leaf':
          np.vx += Math.sin(p.life * 0.1) * 0.05;
          break;
        case 'bubbles':
          np.vx += Math.sin(p.life * 0.2) * 0.03;
          np.vy *= 0.98;
          break;
        case 'sparks':
          np.vy += 0.05;
          np.vx *= 0.95;
          break;
        case 'spores':
          np.vx += Math.sin(p.life * 0.08) * 0.02;
          np.vy += Math.cos(p.life * 0.08) * 0.01;
          break;
        case 'embers':
          np.vx += (Math.random() - 0.5) * 0.1;
          np.vy -= 0.01;
          break;
        case 'glitch':
          if (Math.random() < 0.3) {
            np.x += (Math.random() - 0.5) * 4;
            np.y += (Math.random() - 0.5) * 4;
          }
          break;
        case 'ice':
          np.vx += Math.sin(p.life * 0.15) * 0.02;
          break;
        case 'hearts':
          np.vx += Math.sin(p.life * 0.12) * 0.03;
          np.vy *= 0.99;
          break;
        case 'dust_mote':
          // Gentle floating bob using sin wave
          np.vy = Math.sin(Date.now() * 0.001 + p.y * 0.1) * 0.08;
          np.vx = Math.sin(Date.now() * 0.0008 + p.x * 0.1) * 0.05;
          break;
        case 'steam':
          // Rise and drift sideways with sin wave, fade out
          np.vx = Math.sin(p.life * 0.15) * 0.2;
          np.vy *= 0.99;
          if (np.alpha !== undefined) np.alpha = Math.max(0, np.alpha - 0.008);
          break;
        case 'water_bubble':
          // Rise straight up
          np.vx = 0;
          break;
      }
      if (np.life > 0) {
        updatedParticles.push(np);
      }
    }
    for (const np of newParticles) {
      updatedParticles.push(np);
    }

    // ── Player camera follow & walking flag clear ──
    let newCamX = cameraX;
    let newCamY = cameraY;
    let newZoom = zoom;

    if (player) {
      // Only reset player to idle if movePlayer wasn't called this frame.
      // movePlayer sets _playerMovedThisFrame = true; tick clears it.
      const pAgent = next.get('player');
      if (pAgent && pAgent.anim.state === 'walk' && !(globalThis as any).__choruz_player_moved) {
        next.set('player', {
          ...pAgent,
          anim: { ...pAgent.anim, state: 'idle', ticksInState: 0 },
        });
        changed = true;
      }
      (globalThis as any).__choruz_player_moved = false;

      // Smooth camera lerp following player (Stardew Valley style)
      if (displayWidth && displayHeight) {
        const targetCamX = player.x - (displayWidth / zoom) / 2;
        const targetCamY = player.y - (displayHeight / zoom) / 2;
        newCamX += (targetCamX - newCamX) * CAMERA_LERP;
        newCamY += (targetCamY - newCamY) * CAMERA_LERP;
      }

      // Smooth zoom lerp
      if (targetZoom !== undefined) {
        newZoom += (targetZoom - newZoom) * 0.05;
      }
    } else {
      // Original camera easing (no player)
      const cdx = targetCameraX - cameraX;
      const cdy = targetCameraY - cameraY;
      const camMoved = Math.abs(cdx) > 0.5 || Math.abs(cdy) > 0.5;
      if (camMoved) {
        newCamX = cameraX + cdx * CAMERA_LERP;
        newCamY = cameraY + cdy * CAMERA_LERP;
      } else if (Math.abs(cdx) > 0.01 || Math.abs(cdy) > 0.01) {
        newCamX = targetCameraX;
        newCamY = targetCameraY;
      }
    }

    const updates: Partial<PixelWorldState> = {};
    if (changed) updates.agents = next;
    updates.cameraX = newCamX;
    updates.cameraY = newCamY;
    updates.zoom = newZoom;
    updates.particles = updatedParticles;
    updates.timeOfDay = newTimeOfDay;
    updates.frameCount = newFrameCount;

    set(updates);
  },

  // ─── togglePanel ────────────────────────────────────────────────────────
  togglePanel() {
    set((s) => ({ isOpen: !s.isOpen }));
  },

  // ─── setCameraPosition ──────────────────────────────────────────────────
  setCameraPosition(x, y) {
    set({ cameraX: x, cameraY: y, targetCameraX: x, targetCameraY: y });
  },

  // ─── setWalkabilityMask ────────────────────────────────────────────────
  // Called from MainScene once the active floor's mask PNG is loaded into a
  // CanvasTexture. Used by `handleMessage` for agent A* pathfinding so the
  // agents respect the same visible walls the player does.
  setWalkabilityMask(pixels, width, height) {
    const mask = { pixels, width, height };
    const { agents, worldWidth, worldHeight, walkabilityMask } = get();
    const updates: Partial<PixelWorldState> = { walkabilityMask: mask };

    if (!walkabilityMask && worldWidth > 0 && worldHeight > 0) {
      const nextAgents = new Map(agents);
      let changed = false;

      for (const [id, agent] of agents) {
        if (id === 'player') continue;

        const homeSceneX = (agent.homeX / worldWidth) * mask.width;
        const homeSceneY = (agent.homeY / worldHeight) * mask.height;
        if (isSafeWalkableMaskPoint(mask, homeSceneX, homeSceneY)) continue;

        const spawn = pickInitialAgentSpawnWorld(id, worldWidth, worldHeight, mask);
        nextAgents.set(id, {
          ...agent,
          homeX: spawn.x,
          homeY: spawn.y,
          anim: {
            ...agent.anim,
            x: spawn.x,
            y: spawn.y,
            targetX: spawn.x,
            targetY: spawn.y,
            path: undefined,
            state: 'idle',
            ticksInState: 0,
          },
        });
        changed = true;
      }

      if (changed) updates.agents = nextAgents;
    }

    set(updates);
  },

  // ─── focusHouse ─────────────────────────────────────────────────────────
  focusHouse(houseId) {
    const house = get().houses.get(houseId);
    if (!house) return;

    const centerX = house.worldX + (house.room.width * TILE_SIZE) / 2;
    const centerY = house.worldY + (house.room.height * TILE_SIZE) / 2;

    set({ targetCameraX: centerX, targetCameraY: centerY });
  },

  // ─── setZoom ───────────────────────────────────────────────────────────
  setZoom(newZoom: number) {
    const clamped = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, newZoom));
    set({ zoom: clamped });
  },

  // ─── panCamera ─────────────────────────────────────────────────────────
  panCamera(dx: number, dy: number) {
    const { targetCameraX, targetCameraY } = get();
    set({
      targetCameraX: targetCameraX + dx,
      targetCameraY: targetCameraY + dy,
    });
  },

  // ─── centerCamera ──────────────────────────────────────────────────────
  centerCamera() {
    const { worldWidth, worldHeight } = get();
    set({
      targetCameraX: worldWidth / 2,
      targetCameraY: worldHeight / 2,
    });
  },
}));

if (typeof window !== 'undefined' && process.env.NODE_ENV !== 'production') {
  (window as any).__pixelWorldStore = usePixelWorldStore;
}
