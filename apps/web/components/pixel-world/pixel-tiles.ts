// pixel-tiles.ts — Office-themed tile system for the pixel world.
// Defines tile types for an office building floor plan (carpet, wood,
// kitchen tile, hallway, wall, glass, door), builds a tile atlas using
// PNG sprite sheet assets (Ninja Adventure tilesets) with fillRect fallback,
// generates a world grid from the office layout, and provides chunk-based
// cached rendering.
// Pure module — no React dependencies.

// ---------------------------------------------------------------------------
// Tile type enum
// ---------------------------------------------------------------------------

export const enum Tile {
  /** Void / empty space outside the office. */
  VOID = 0,
  /** Navy carpet — woven texture variant A. */
  CARPET_A = 1,
  /** Navy carpet — woven texture variant B. */
  CARPET_B = 2,
  /** Warm wood floor with plank grain. */
  WOOD_FLOOR = 3,
  /** Black & white checkerboard kitchen tile. */
  KITCHEN_TILE = 4,
  /** Grey concrete hallway. */
  HALLWAY = 5,
  /** Solid dark wall (base type — replaced by auto-tiled variants after layout). */
  WALL = 6,
  /** Semi-transparent glass wall. */
  GLASS_WALL = 7,
  /** Wooden door (hallway meets room). */
  DOOR = 8,
  /** Window on exterior wall (sky-blue pane + frame). */
  WINDOW = 9,
  /** Concrete exterior / void boundary. */
  CONCRETE = 10,

  // ── Auto-tiled wall variants (bitmask-selected) ──
  WALL_SINGLE = 11,
  WALL_TOP_END = 12,
  WALL_BOTTOM_END = 13,
  WALL_LEFT_END = 14,
  WALL_RIGHT_END = 15,
  WALL_VERTICAL = 16,
  WALL_HORIZONTAL = 17,
  WALL_TOP_LEFT_CORNER = 18,
  WALL_TOP_RIGHT_CORNER = 19,
  WALL_BOTTOM_LEFT_CORNER = 20,
  WALL_BOTTOM_RIGHT_CORNER = 21,
  WALL_T_DOWN = 22,
  WALL_T_UP = 23,
  WALL_T_LEFT = 24,
  WALL_T_RIGHT = 25,
  WALL_CROSS = 26,

  // ── Phase 1: Building Shell tiles ──
  /** Wall-floor trim/molding at the base of walls (Stardew-style). */
  WALL_TRIM = 27,
  /** Front/south wall cutaway — translucent wall top edge (dollhouse style). */
  FRONT_WALL_CUTAWAY = 28,
  /** Exterior ground — dark asphalt/concrete outside the building boundary. */
  EXTERIOR_GROUND = 29,
  /** Exterior wall — darker, thicker perimeter wall. */
  EXTERIOR_WALL = 30,
  /** Exterior wall top — visible wall height for back walls (2nd tile). */
  WALL_BACK_TOP = 31,
}

/** Total number of tile types. */
export const TILE_COUNT = 32;

/** Tile size in pixels. */
export const TILE_SIZE = 16;

/** Number of columns in the atlas spritesheet. */
const ATLAS_COLS = 4;

// ---------------------------------------------------------------------------
// Seeded PRNG for deterministic texture
// ---------------------------------------------------------------------------

function mulberry32(seed: number): () => number {
  let s = seed | 0;
  return () => {
    s = (s + 0x6d2b79f5) | 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// ---------------------------------------------------------------------------
// PNG Tile Sheet — loads real Ninja Adventure tilesets for tile rendering
// ---------------------------------------------------------------------------

/** Loaded PNG images for tile rendering. */
let floorSheetImg: HTMLImageElement | null = null;
let wallSheetImg: HTMLImageElement | null = null;
let pngTilesReady = false;
let pngLoadAttempted = false;

/** Monotonically increasing version counter — bumped whenever the tile atlas
 *  changes (e.g. PNG sheets loaded). Consumers should compare against their own
 *  snapshot and rebuild chunk caches when the value changes. */
let tileAtlasVersion = 0;

/** Return the current atlas version. */
export function getTileAtlasVersion(): number {
  return tileAtlasVersion;
}

/**
 * Load the PNG tile sheets. Called once during initialization.
 * Non-blocking — tiles fall back to procedural atlas until PNGs are ready.
 */
export function loadTileSheets(): Promise<void> {
  if (pngLoadAttempted) return Promise.resolve();
  pngLoadAttempted = true;

  const loadImg = (src: string): Promise<HTMLImageElement> =>
    new Promise((resolve, reject) => {
      const img = new Image();
      img.onload = () => resolve(img);
      img.onerror = () => reject(new Error(`Failed to load: ${src}`));
      img.src = src;
    });

  return Promise.all([
    loadImg('/sprites/tilesets/office-floor.png'),
    loadImg('/sprites/tilesets/office-walls.png'),
  ]).then(([floor, walls]) => {
    floorSheetImg = floor;
    wallSheetImg = walls;
    pngTilesReady = true;
    // Invalidate the procedural atlas so next getTileAtlas() rebuilds with PNGs
    tileAtlas = null;
    // Bump version so the store knows to recreate the chunk cache
    tileAtlasVersion++;
  }).catch(err => {
    console.warn('[pixel-tiles] PNG tile sheets failed to load, using procedural fallback:', err.message);
  });
}

/** Whether PNG tile sheets are loaded and ready. */
export function arePngTilesReady(): boolean {
  return pngTilesReady;
}

// ---------------------------------------------------------------------------
// PNG source rect mappings — maps Tile enum to (col, row) in the PNG sheets
// ---------------------------------------------------------------------------

const PNG_FLOOR_MAP: Partial<Record<number, [number, number]>> = {
  [Tile.VOID]: [20, 16],
  [Tile.WOOD_FLOOR]: [1, 5],
  [Tile.KITCHEN_TILE]: [12, 5],
  [Tile.HALLWAY]: [1, 13],
  [Tile.CONCRETE]: [18, 13],
  [Tile.DOOR]: [2, 5],
  [Tile.EXTERIOR_GROUND]: [19, 15],
  [Tile.WALL_TRIM]: [3, 13],
  [Tile.FRONT_WALL_CUTAWAY]: [0, 13],
};

const PNG_WALL_MAP: Partial<Record<number, [number, number]>> = {
  [Tile.WALL]: [2, 2],
  [Tile.WALL_SINGLE]: [0, 0],
  [Tile.WALL_TOP_END]: [2, 0],
  [Tile.WALL_BOTTOM_END]: [2, 4],
  [Tile.WALL_LEFT_END]: [0, 2],
  [Tile.WALL_RIGHT_END]: [4, 2],
  [Tile.WALL_VERTICAL]: [0, 1],
  [Tile.WALL_HORIZONTAL]: [1, 0],
  [Tile.WALL_TOP_LEFT_CORNER]: [0, 0],
  [Tile.WALL_TOP_RIGHT_CORNER]: [4, 0],
  [Tile.WALL_BOTTOM_LEFT_CORNER]: [0, 4],
  [Tile.WALL_BOTTOM_RIGHT_CORNER]: [4, 4],
  [Tile.WALL_T_DOWN]: [2, 0],
  [Tile.WALL_T_UP]: [2, 4],
  [Tile.WALL_T_LEFT]: [4, 2],
  [Tile.WALL_T_RIGHT]: [0, 2],
  [Tile.WALL_CROSS]: [2, 2],
  [Tile.GLASS_WALL]: [7, 2],
  [Tile.WINDOW]: [7, 1],
  [Tile.EXTERIOR_WALL]: [2, 7],
  [Tile.WALL_BACK_TOP]: [2, 1],
};

// ---------------------------------------------------------------------------
// Tile atlas — PNG-backed with procedural fallback
// ---------------------------------------------------------------------------

let tileAtlas: OffscreenCanvas | HTMLCanvasElement | null = null;

/** Get or create the tile atlas. Thread-safe singleton. */
export function getTileAtlas(): OffscreenCanvas | HTMLCanvasElement {
  if (tileAtlas) return tileAtlas;
  tileAtlas = buildTileAtlas();
  return tileAtlas;
}

/** Returns the (col, row) position of a tile in the atlas. */
function tileAtlasPos(tile: Tile): { ax: number; ay: number } {
  return {
    ax: (tile % ATLAS_COLS) * TILE_SIZE,
    ay: Math.floor(tile / ATLAS_COLS) * TILE_SIZE,
  };
}

/** Draw a single tile from the atlas onto a destination context. */
export function drawTile(
  dst: CanvasRenderingContext2D,
  tile: Tile,
  dx: number,
  dy: number,
  scale: number = 1,
): void {
  const atlas = getTileAtlas();
  const { ax, ay } = tileAtlasPos(tile);
  dst.drawImage(
    atlas as any,
    ax, ay, TILE_SIZE, TILE_SIZE,
    dx, dy, TILE_SIZE * scale, TILE_SIZE * scale,
  );
}

// ---------------------------------------------------------------------------
// Atlas builder — uses PNG sprites when available, procedural fallback
// ---------------------------------------------------------------------------

function buildTileAtlas(): OffscreenCanvas | HTMLCanvasElement {
  const rows = Math.ceil(TILE_COUNT / ATLAS_COLS);
  const width = ATLAS_COLS * TILE_SIZE;
  const height = rows * TILE_SIZE;

  let canvas: OffscreenCanvas | HTMLCanvasElement;
  if (typeof OffscreenCanvas !== 'undefined') {
    canvas = new OffscreenCanvas(width, height);
  } else {
    canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
  }

  const ctx = canvas.getContext('2d') as OffscreenCanvasRenderingContext2D | CanvasRenderingContext2D;
  if (!ctx) return canvas;

  (ctx as any).imageSmoothingEnabled = false;

  for (let t = 0; t < TILE_COUNT; t++) {
    const { ax, ay } = tileAtlasPos(t as Tile);
    let drawnFromPng = false;

    if (pngTilesReady) {
      const floorSrc = PNG_FLOOR_MAP[t];
      if (floorSrc && floorSheetImg) {
        ctx.drawImage(
          floorSheetImg,
          floorSrc[0] * TILE_SIZE, floorSrc[1] * TILE_SIZE,
          TILE_SIZE, TILE_SIZE,
          ax, ay,
          TILE_SIZE, TILE_SIZE,
        );
        drawnFromPng = true;
      }

      if (!drawnFromPng) {
        const wallSrc = PNG_WALL_MAP[t];
        if (wallSrc && wallSheetImg) {
          ctx.drawImage(
            wallSheetImg,
            wallSrc[0] * TILE_SIZE, wallSrc[1] * TILE_SIZE,
            TILE_SIZE, TILE_SIZE,
            ax, ay,
            TILE_SIZE, TILE_SIZE,
          );
          drawnFromPng = true;
        }
      }
    }

    if (!drawnFromPng) {
      ctx.save();
      ctx.translate(ax, ay);
      paintTile(ctx as CanvasRenderingContext2D, t as Tile);
      ctx.restore();
    }
  }

  return canvas;
}

/** Paint a single 16x16 tile at origin (0,0). Uses ONLY fillRect. */
function paintTile(ctx: CanvasRenderingContext2D, tile: Tile): void {
  switch (tile) {
    case Tile.VOID:
      paintVoidTile(ctx);
      break;
    case Tile.CARPET_A:
      paintCarpetTile(ctx, 0);
      break;
    case Tile.CARPET_B:
      paintCarpetTile(ctx, 1);
      break;
    case Tile.WOOD_FLOOR:
      paintWoodFloorTile(ctx);
      break;
    case Tile.KITCHEN_TILE:
      paintKitchenTile(ctx);
      break;
    case Tile.HALLWAY:
      paintHallwayTile(ctx);
      break;
    case Tile.WALL:
      paintWallTile(ctx);
      break;
    case Tile.GLASS_WALL:
      paintGlassWallTile(ctx);
      break;
    case Tile.DOOR:
      paintDoorTile(ctx);
      break;
    case Tile.WINDOW:
      paintWindowTile(ctx);
      break;
    case Tile.CONCRETE:
      paintConcreteTile(ctx);
      break;
    case Tile.WALL_TRIM:
      paintWallTrimTile(ctx);
      break;
    case Tile.FRONT_WALL_CUTAWAY:
      paintFrontWallCutawayTile(ctx);
      break;
    case Tile.EXTERIOR_GROUND:
      paintExteriorGroundTile(ctx);
      break;
    case Tile.EXTERIOR_WALL:
      paintExteriorWallTile(ctx);
      break;
    case Tile.WALL_BACK_TOP:
      paintWallBackTopTile(ctx);
      break;
    case Tile.WALL_SINGLE:
    case Tile.WALL_TOP_END:
    case Tile.WALL_BOTTOM_END:
    case Tile.WALL_LEFT_END:
    case Tile.WALL_RIGHT_END:
    case Tile.WALL_VERTICAL:
    case Tile.WALL_HORIZONTAL:
    case Tile.WALL_TOP_LEFT_CORNER:
    case Tile.WALL_TOP_RIGHT_CORNER:
    case Tile.WALL_BOTTOM_LEFT_CORNER:
    case Tile.WALL_BOTTOM_RIGHT_CORNER:
    case Tile.WALL_T_DOWN:
    case Tile.WALL_T_UP:
    case Tile.WALL_T_LEFT:
    case Tile.WALL_T_RIGHT:
    case Tile.WALL_CROSS:
      paintAutoWallTile(ctx, tile);
      break;
  }
}

// ---------------------------------------------------------------------------
// Individual tile painters (all draw within 0,0 -> 16,16, fillRect ONLY)
// ---------------------------------------------------------------------------

function paintVoidTile(ctx: CanvasRenderingContext2D): void {
  ctx.fillStyle = '#0f0f14';
  ctx.fillRect(0, 0, 16, 16);
  const rng = mulberry32(7919);
  for (let i = 0; i < 8; i++) {
    const x = Math.floor(rng() * 16);
    const y = Math.floor(rng() * 16);
    ctx.fillStyle = rng() > 0.5 ? '#2a2a35' : '#1a1a24';
    ctx.fillRect(x, y, 1, 1);
  }
}

function paintCarpetTile(ctx: CanvasRenderingContext2D, variant: number): void {
  const isA = variant === 0;
  const base = isA ? '#2b4c7e' : '#7e2b35';
  const dark = isA ? '#1e365c' : '#5c1e25';
  const light = isA ? '#3a609c' : '#9c3a45';

  ctx.fillStyle = base;
  ctx.fillRect(0, 0, 16, 16);

  for (let y = 0; y < 16; y++) {
    for (let x = 0; x < 16; x++) {
      if ((x + y) % 2 === 0) {
        ctx.fillStyle = light;
        ctx.fillRect(x, y, 1, 1);
      } else if ((x * y) % 3 === 0) {
        ctx.fillStyle = dark;
        ctx.fillRect(x, y, 1, 1);
      }
    }
  }
}

function paintWoodFloorTile(ctx: CanvasRenderingContext2D): void {
  ctx.fillStyle = '#c08147';
  ctx.fillRect(0, 0, 16, 16);

  for (let x = 0; x < 16; x += 4) {
    ctx.fillStyle = '#965b27';
    ctx.fillRect(x + 3, 0, 1, 16);
    ctx.fillStyle = '#d49b5b';
    ctx.fillRect(x, 0, 1, 16);
  }

  ctx.fillStyle = '#5e3315';
  ctx.fillRect(0, 3, 4, 1);
  ctx.fillRect(4, 10, 4, 1);
  ctx.fillRect(8, 5, 4, 1);
  ctx.fillRect(12, 12, 4, 1);

  ctx.fillStyle = '#3a1f0d';
  ctx.fillRect(1, 2, 1, 1);
  ctx.fillRect(5, 9, 1, 1);
  ctx.fillRect(9, 4, 1, 1);
  ctx.fillRect(13, 11, 1, 1);
}

function paintKitchenTile(ctx: CanvasRenderingContext2D): void {
  for (let y = 0; y < 16; y += 8) {
    for (let x = 0; x < 16; x += 8) {
      const isWhite = (x + y) % 16 === 0;
      ctx.fillStyle = isWhite ? '#f4f4f4' : '#3b3b42';
      ctx.fillRect(x, y, 8, 8);

      ctx.fillStyle = isWhite ? '#ffffff' : '#52525a';
      ctx.fillRect(x, y, 8, 1);
      ctx.fillRect(x, y, 1, 8);

      ctx.fillStyle = isWhite ? '#dcdcdc' : '#26262b';
      ctx.fillRect(x, y + 7, 8, 1);
      ctx.fillRect(x + 7, y, 1, 8);
    }
  }

  ctx.fillStyle = '#8b8b94';
  ctx.fillRect(7, 0, 2, 16);
  ctx.fillRect(0, 7, 16, 2);
}

function paintHallwayTile(ctx: CanvasRenderingContext2D): void {
  ctx.fillStyle = '#7b828c';
  ctx.fillRect(0, 0, 16, 16);

  for (let y = 0; y < 16; y += 8) {
    for (let x = 0; x < 16; x += 8) {
      ctx.fillStyle = '#929aa6';
      ctx.fillRect(x, y, 8, 1);
      ctx.fillRect(x, y, 1, 8);

      ctx.fillStyle = '#5d646e';
      ctx.fillRect(x, y + 7, 8, 1);
      ctx.fillRect(x + 7, y, 1, 8);
    }
  }
}

function paintWallTile(ctx: CanvasRenderingContext2D): void {
  ctx.fillStyle = '#d1c7a3';
  ctx.fillRect(0, 0, 16, 16);

  ctx.fillStyle = '#c2b793';
  for (let x = 2; x < 16; x += 4) {
    ctx.fillRect(x, 0, 1, 16);
  }

  ctx.fillStyle = '#7a4b24';
  ctx.fillRect(0, 13, 16, 3);
  ctx.fillStyle = '#965b27';
  ctx.fillRect(0, 13, 16, 1);
}

function paintAutoWallTile(ctx: CanvasRenderingContext2D, tile: Tile): void {
  ctx.fillStyle = '#5c6370';
  ctx.fillRect(0, 0, 16, 16);

  const hasTop = (
    tile === Tile.WALL_BOTTOM_END || tile === Tile.WALL_VERTICAL ||
    tile === Tile.WALL_BOTTOM_LEFT_CORNER || tile === Tile.WALL_BOTTOM_RIGHT_CORNER ||
    tile === Tile.WALL_T_LEFT || tile === Tile.WALL_T_RIGHT ||
    tile === Tile.WALL_T_UP || tile === Tile.WALL_CROSS
  );
  const hasLeft = (
    tile === Tile.WALL_RIGHT_END || tile === Tile.WALL_HORIZONTAL ||
    tile === Tile.WALL_TOP_RIGHT_CORNER || tile === Tile.WALL_BOTTOM_RIGHT_CORNER ||
    tile === Tile.WALL_T_UP || tile === Tile.WALL_T_DOWN ||
    tile === Tile.WALL_T_RIGHT || tile === Tile.WALL_CROSS
  );
  const hasBottom = (
    tile === Tile.WALL_TOP_END || tile === Tile.WALL_VERTICAL ||
    tile === Tile.WALL_TOP_LEFT_CORNER || tile === Tile.WALL_TOP_RIGHT_CORNER ||
    tile === Tile.WALL_T_LEFT || tile === Tile.WALL_T_RIGHT ||
    tile === Tile.WALL_T_DOWN || tile === Tile.WALL_CROSS
  );
  const hasRight = (
    tile === Tile.WALL_LEFT_END || tile === Tile.WALL_HORIZONTAL ||
    tile === Tile.WALL_TOP_LEFT_CORNER || tile === Tile.WALL_BOTTOM_LEFT_CORNER ||
    tile === Tile.WALL_T_UP || tile === Tile.WALL_T_DOWN ||
    tile === Tile.WALL_T_LEFT || tile === Tile.WALL_CROSS
  );

  ctx.fillStyle = '#4b515c';
  ctx.fillRect(4, 4, 8, 8);

  if (!hasTop) {
    ctx.fillStyle = '#828a99';
    ctx.fillRect(0, 0, 16, 2);
  }
  if (!hasLeft) {
    ctx.fillStyle = '#828a99';
    ctx.fillRect(0, 0, 2, 16);
  }
  if (!hasBottom) {
    ctx.fillStyle = '#3e434d';
    ctx.fillRect(0, 14, 16, 2);
  }
  if (!hasRight) {
    ctx.fillStyle = '#3e434d';
    ctx.fillRect(14, 0, 2, 16);
  }

  if (!hasTop && !hasLeft) {
    ctx.fillStyle = '#9ba3b3';
    ctx.fillRect(0, 0, 2, 2);
  }
  if (!hasBottom && !hasRight) {
    ctx.fillStyle = '#2a2e36';
    ctx.fillRect(14, 14, 2, 2);
  }
}

function paintWindowTile(ctx: CanvasRenderingContext2D): void {
  ctx.fillStyle = '#a84b3e';
  ctx.fillRect(0, 0, 16, 16);

  ctx.fillStyle = '#e6d9c3';
  ctx.fillRect(2, 2, 12, 12);

  ctx.fillStyle = '#639bff';
  ctx.fillRect(4, 4, 8, 8);

  ctx.fillStyle = '#e6d9c3';
  ctx.fillRect(7, 4, 2, 8);
  ctx.fillRect(4, 7, 8, 2);

  ctx.fillStyle = '#b5d6ff';
  ctx.fillRect(5, 5, 2, 2);
  ctx.fillRect(9, 9, 2, 2);
}

function paintGlassWallTile(ctx: CanvasRenderingContext2D): void {
  ctx.fillStyle = '#4b515c';
  ctx.fillRect(0, 0, 16, 16);

  ctx.fillStyle = '#8ab1d4';
  ctx.fillRect(2, 2, 12, 12);

  ctx.fillStyle = '#c2e0f5';
  ctx.fillRect(4, 4, 8, 1);
  ctx.fillRect(4, 5, 1, 7);
  ctx.fillRect(10, 10, 2, 1);
}

function paintDoorTile(ctx: CanvasRenderingContext2D): void {
  ctx.fillStyle = '#4a3018';
  ctx.fillRect(0, 0, 16, 16);

  ctx.fillStyle = '#7a4b24';
  ctx.fillRect(2, 2, 12, 14);

  ctx.fillStyle = '#5e3615';
  ctx.fillRect(4, 4, 8, 4);
  ctx.fillRect(4, 10, 8, 4);

  ctx.fillStyle = '#e8c170';
  ctx.fillRect(11, 8, 2, 2);
  ctx.fillStyle = '#b59145';
  ctx.fillRect(12, 9, 1, 1);
}

function paintConcreteTile(ctx: CanvasRenderingContext2D): void {
  ctx.fillStyle = '#6b7280';
  ctx.fillRect(0, 0, 16, 16);

  const rng = mulberry32(3337);
  for (let i = 0; i < 15; i++) {
    const x = Math.floor(rng() * 16);
    const y = Math.floor(rng() * 16);
    ctx.fillStyle = rng() > 0.5 ? '#9ca3af' : '#4b5563';
    ctx.fillRect(x, y, 1, 1);
  }

  ctx.fillStyle = '#374151';
  ctx.fillRect(2, 2, 1, 1);
  ctx.fillRect(3, 3, 2, 1);
  ctx.fillRect(4, 4, 1, 2);
}

function paintWallTrimTile(ctx: CanvasRenderingContext2D): void {
  ctx.fillStyle = '#7b828c';
  ctx.fillRect(0, 0, 16, 16);

  ctx.fillStyle = '#7a4b24';
  ctx.fillRect(0, 0, 16, 4);
  ctx.fillStyle = '#965b27';
  ctx.fillRect(0, 0, 16, 1);
  ctx.fillStyle = '#4a2e15';
  ctx.fillRect(0, 3, 16, 1);
}

function paintFrontWallCutawayTile(ctx: CanvasRenderingContext2D): void {
  ctx.fillStyle = '#7b828c';
  ctx.fillRect(0, 0, 16, 16);

  ctx.fillStyle = '#382210';
  ctx.fillRect(0, 0, 16, 3);
  ctx.fillStyle = '#5e3a1f';
  ctx.fillRect(0, 0, 16, 1);
}

function paintExteriorGroundTile(ctx: CanvasRenderingContext2D): void {
  ctx.fillStyle = '#3d2e20';
  ctx.fillRect(0, 0, 16, 16);

  const rng = mulberry32(8819);
  for (let i = 0; i < 20; i++) {
    const x = Math.floor(rng() * 16);
    const y = Math.floor(rng() * 16);
    ctx.fillStyle = rng() > 0.5 ? '#523e2b' : '#291e14';
    ctx.fillRect(x, y, rng() > 0.8 ? 2 : 1, 1);
  }
}

function paintExteriorWallTile(ctx: CanvasRenderingContext2D): void {
  ctx.fillStyle = '#a84b3e';
  ctx.fillRect(0, 0, 16, 16);

  ctx.fillStyle = '#c4b1a1';
  for (let y = 3; y < 16; y += 4) {
    ctx.fillRect(0, y, 16, 1);
  }
  for (let y = 0; y < 16; y += 4) {
    const offset = (y % 8 === 0) ? 4 : 0;
    for (let x = offset; x < 16; x += 8) {
      ctx.fillRect(x, y, 1, 3);
    }
  }

  for (let y = 0; y < 16; y += 4) {
    const offset = (y % 8 === 0) ? 0 : -4;
    for (let x = offset; x < 16; x += 8) {
      if (x >= 0) {
        ctx.fillStyle = '#c26255';
        ctx.fillRect(x + 1, y, 6, 1);
        ctx.fillStyle = '#7a3025';
        ctx.fillRect(x + 1, y + 2, 6, 1);
      }
    }
  }
}

function paintWallBackTopTile(ctx: CanvasRenderingContext2D): void {
  ctx.fillStyle = '#2e1e12';
  ctx.fillRect(0, 0, 16, 16);
  ctx.fillStyle = '#4a301d';
  ctx.fillRect(0, 0, 16, 2);
  ctx.fillStyle = '#1a100a';
  ctx.fillRect(0, 14, 16, 2);
}

// ---------------------------------------------------------------------------
// World grid types
// ---------------------------------------------------------------------------

export const enum Layer {
  GROUND = 0,
}

export const LAYER_COUNT = 1;

export interface TileGrid {
  cols: number;
  rows: number;
  layers: Int8Array[];
}

// ---------------------------------------------------------------------------
// Office layout types (produced by pixel-world-store.ts)
// ---------------------------------------------------------------------------

export interface OfficeRoom {
  id: string;
  memberCount: number;
  tileX: number;
  tileY: number;
  width: number;
  height: number;
  roomType: 'conference' | 'workspace' | 'lounge' | 'server' | 'kitchen' | 'reception' | 'manager' | 'bathroom';
  doorTileX: number;
  doorTileY: number;
  interactionPoints: { tileX: number; tileY: number; type: string }[];
}

export interface OfficeLayout {
  rooms: OfficeRoom[];
  /** Flat Uint8Array grid (row-major): grid[row * gridWidth + col]. */
  grid: Uint8Array;
  gridWidth: number;
  gridHeight: number;
}

// ---------------------------------------------------------------------------
// World grid generation from an office layout
// ---------------------------------------------------------------------------

export function generateWorldGrid(layout: OfficeLayout): TileGrid {
  const cols = layout.gridWidth;
  const rows = layout.gridHeight;

  const layers: Int8Array[] = [];
  for (let i = 0; i < LAYER_COUNT; i++) {
    layers.push(new Int8Array(cols * rows).fill(-1));
  }

  const ground = layers[Layer.GROUND];

  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      const idx = r * cols + c;
      const layoutTile = (r < layout.gridHeight && c < layout.gridWidth)
        ? layout.grid[r * layout.gridWidth + c]
        : 0;

      switch (layoutTile) {
        case 0:
          ground[idx] = Tile.VOID;
          break;
        case 1:
          ground[idx] = (c + r) % 2 === 0 ? Tile.CARPET_A : Tile.CARPET_B;
          break;
        case 2:
          ground[idx] = Tile.WOOD_FLOOR;
          break;
        case 3:
          ground[idx] = Tile.KITCHEN_TILE;
          break;
        case 4:
          ground[idx] = Tile.HALLWAY;
          break;
        case 5:
          ground[idx] = Tile.WALL;
          break;
        case 6:
          ground[idx] = Tile.GLASS_WALL;
          break;
        case 7:
          ground[idx] = Tile.DOOR;
          break;
        case 8:
          ground[idx] = Tile.WINDOW;
          break;
        case 9:
          ground[idx] = Tile.CONCRETE;
          break;
        case 10:
          ground[idx] = Tile.EXTERIOR_GROUND;
          break;
        case 11:
          ground[idx] = Tile.EXTERIOR_WALL;
          break;
        case 12:
          ground[idx] = Tile.WALL_TRIM;
          break;
        case 13:
          ground[idx] = Tile.FRONT_WALL_CUTAWAY;
          break;
        case 14:
          ground[idx] = Tile.WALL_BACK_TOP;
          break;
        default:
          ground[idx] = Tile.EXTERIOR_GROUND;
          break;
      }
    }
  }

  const tempGrid = new Int8Array(ground);

  const getTileAt = (c: number, r: number): number => {
    if (c < 0 || c >= cols || r < 0 || r >= rows) return Tile.VOID;
    return tempGrid[r * cols + c];
  };

  const isWallLike = (t: number): boolean =>
    t === Tile.WALL || (t >= Tile.WALL_SINGLE && t <= Tile.WALL_CROSS) ||
    t === Tile.WINDOW || t === Tile.EXTERIOR_WALL;

  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      const idx = r * cols + c;
      if (tempGrid[idx] !== Tile.WALL) continue;

      const up = isWallLike(getTileAt(c, r - 1)) ? 1 : 0;
      const down = isWallLike(getTileAt(c, r + 1)) ? 1 : 0;
      const left = isWallLike(getTileAt(c - 1, r)) ? 1 : 0;
      const right = isWallLike(getTileAt(c + 1, r)) ? 1 : 0;

      const mask = up | (down << 1) | (left << 2) | (right << 3);

      switch (mask) {
        case 0:  ground[idx] = Tile.WALL_SINGLE; break;
        case 1:  ground[idx] = Tile.WALL_BOTTOM_END; break;
        case 2:  ground[idx] = Tile.WALL_TOP_END; break;
        case 3:  ground[idx] = Tile.WALL_VERTICAL; break;
        case 4:  ground[idx] = Tile.WALL_RIGHT_END; break;
        case 5:  ground[idx] = Tile.WALL_BOTTOM_RIGHT_CORNER; break;
        case 6:  ground[idx] = Tile.WALL_TOP_RIGHT_CORNER; break;
        case 7:  ground[idx] = Tile.WALL_T_LEFT; break;
        case 8:  ground[idx] = Tile.WALL_LEFT_END; break;
        case 9:  ground[idx] = Tile.WALL_BOTTOM_LEFT_CORNER; break;
        case 10: ground[idx] = Tile.WALL_TOP_LEFT_CORNER; break;
        case 11: ground[idx] = Tile.WALL_T_UP; break;
        case 12: ground[idx] = Tile.WALL_HORIZONTAL; break;
        case 13: ground[idx] = Tile.WALL_T_DOWN; break;
        case 14: ground[idx] = Tile.WALL_T_RIGHT; break;
        case 15: ground[idx] = Tile.WALL_CROSS; break;
        default: break;
      }
    }
  }

  return { cols, rows, layers };
}

// ---------------------------------------------------------------------------
// Chunk-based rendering cache
// ---------------------------------------------------------------------------

export const CHUNK_SIZE = 10;

/** Maximum number of cached chunks. LRU eviction via Map insertion order (Fix 3). */
const MAX_CHUNKS = 100;

export interface ChunkCache {
  chunks: Map<string, OffscreenCanvas | HTMLCanvasElement>;
  gridId: number;
}

let gridIdCounter = 0;

export function createChunkCache(): ChunkCache {
  gridIdCounter++;
  return {
    chunks: new Map(),
    gridId: gridIdCounter,
  };
}

function getChunk(
  cache: ChunkCache,
  grid: TileGrid,
  chunkCol: number,
  chunkRow: number,
  scale: number,
): OffscreenCanvas | HTMLCanvasElement | null {
  const key = `${cache.gridId}:${chunkCol}:${chunkRow}:${scale}`;
  const existing = cache.chunks.get(key);
  if (existing) {
    // Move to end of Map (most recently used) for LRU tracking
    cache.chunks.delete(key);
    cache.chunks.set(key, existing);
    return existing;
  }

  const tilePixelSize = TILE_SIZE * scale;
  const chunkPixelSize = CHUNK_SIZE * tilePixelSize;

  let canvas: OffscreenCanvas | HTMLCanvasElement;
  if (typeof OffscreenCanvas !== 'undefined') {
    canvas = new OffscreenCanvas(chunkPixelSize, chunkPixelSize);
  } else {
    canvas = document.createElement('canvas');
    canvas.width = chunkPixelSize;
    canvas.height = chunkPixelSize;
  }

  const ctx = canvas.getContext('2d') as CanvasRenderingContext2D;
  if (!ctx) return null;
  (ctx as any).imageSmoothingEnabled = false;

  const startCol = chunkCol * CHUNK_SIZE;
  const startRow = chunkRow * CHUNK_SIZE;

  for (let layerIdx = 0; layerIdx < LAYER_COUNT; layerIdx++) {
    const layer = grid.layers[layerIdx];
    for (let dr = 0; dr < CHUNK_SIZE; dr++) {
      for (let dc = 0; dc < CHUNK_SIZE; dc++) {
        const gr = startRow + dr;
        const gc = startCol + dc;
        if (gr >= grid.rows || gc >= grid.cols) continue;

        const tile = layer[gr * grid.cols + gc];
        if (tile < 0) continue;

        drawTile(ctx, tile as Tile, dc * tilePixelSize, dr * tilePixelSize, scale);
      }
    }
  }

  // LRU eviction: if at capacity, evict the least recently used (first entry)
  if (cache.chunks.size >= MAX_CHUNKS) {
    const lruKey = cache.chunks.keys().next().value;
    if (lruKey !== undefined) {
      const evicted = cache.chunks.get(lruKey);
      // Force immediate GPU/CPU memory release (Fix 6 pattern)
      if (evicted) {
        evicted.width = 0;
        evicted.height = 0;
      }
      cache.chunks.delete(lruKey);
    }
  }

  cache.chunks.set(key, canvas);
  return canvas;
}

// ---------------------------------------------------------------------------
// Main tile rendering function — draws visible tiles using chunk cache
// ---------------------------------------------------------------------------

export function renderTileGrid(
  ctx: CanvasRenderingContext2D,
  grid: TileGrid,
  cache: ChunkCache,
  cameraX: number,
  cameraY: number,
  canvasWidth: number,
  canvasHeight: number,
  scale: number,
  gridOffsetX: number,
  gridOffsetY: number,
): void {
  const tilePixelSize = TILE_SIZE * scale;
  const chunkPixelSize = CHUNK_SIZE * tilePixelSize;

  const gCamX = cameraX - gridOffsetX;
  const gCamY = cameraY - gridOffsetY;

  const startChunkCol = Math.max(0, Math.floor(gCamX / chunkPixelSize));
  const startChunkRow = Math.max(0, Math.floor(gCamY / chunkPixelSize));
  const totalChunkCols = Math.ceil(grid.cols / CHUNK_SIZE);
  const totalChunkRows = Math.ceil(grid.rows / CHUNK_SIZE);
  const endChunkCol = Math.min(totalChunkCols - 1, Math.ceil((gCamX + canvasWidth) / chunkPixelSize));
  const endChunkRow = Math.min(totalChunkRows - 1, Math.ceil((gCamY + canvasHeight) / chunkPixelSize));

  for (let cr = startChunkRow; cr <= endChunkRow; cr++) {
    for (let cc = startChunkCol; cc <= endChunkCol; cc++) {
      const chunk = getChunk(cache, grid, cc, cr, scale);
      if (!chunk) continue;

      const screenX = cc * chunkPixelSize - gCamX;
      const screenY = cr * chunkPixelSize - gCamY;

      ctx.drawImage(chunk as any, Math.round(screenX), Math.round(screenY));
    }
  }
}

export function getGridPadding(): number {
  return 0;
}

// ---------------------------------------------------------------------------
// Full-map composite renderer (bypasses Phaser tilemap API)
// ---------------------------------------------------------------------------

/**
 * Render the entire tile grid onto a single canvas, bypassing Phaser's
 * tilemap API (which fails with addCanvas textures in Canvas mode).
 */
export function renderFullMapCanvas(tileGrid: TileGrid): HTMLCanvasElement {
  const cols = tileGrid.cols;
  const rows = tileGrid.rows;
  const ground = tileGrid.layers[0]; // Layer.GROUND
  const canvas = document.createElement('canvas');
  canvas.width = cols * TILE_SIZE;
  canvas.height = rows * TILE_SIZE;
  const ctx = canvas.getContext('2d');
  if (!ctx) return canvas;
  ctx.imageSmoothingEnabled = false;

  // Paint each tile directly using paintTile (bypass tile atlas which may have
  // empty slots due to PNG sprite sheets not loading in the Canvas renderer).
  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      const v = ground[r * cols + c];
      if (v >= 0) {
        ctx.save();
        ctx.translate(c * TILE_SIZE, r * TILE_SIZE);
        paintTile(ctx, v as Tile);
        ctx.restore();
      }
    }
  }
  return canvas;
}
