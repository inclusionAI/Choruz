// pixel-houses.ts — Canvas-drawn pixel art office rooms for group conversations.
// Phase 3: Room Interiors with Stardew-level furniture density.
// Uses a fixed hex-code furniture catalog with fillRect pixel-perfect rendering.
// Each room type gets a dedicated furniture layout with anchor furniture, functional
// items, rugs, and a clutter pass ensuring no more than 2-3 empty floor tiles contiguous.
// Standalone pure module: no React dependencies.

import { hashStr } from './pixel-sprites';
import { TILE_SIZE } from './pixel-tiles';
import type { OfficeRoom } from './pixel-tiles';

// ─── Interfaces ─────────────────────────────────────────────────────────────

export interface RoomTemplate {
  width: number;
  height: number;
  pixels: number[][];
  doorX: number;
  doorY: number;
  labelY: number;
}

export interface RoomColors {
  wall: string;
  wallDark: string;
  accent: string;
  accentDark: string;
  floor: string;
  floorDark: string;
  furniture: string;
  furnitureDark: string;
  screen: string;
  screenGlow: string;
  trim: string;
}

export interface GeneratedHouse {
  template: RoomTemplate;
  colors: RoomColors;
}

// ─── Color helpers ──────────────────────────────────────────────────────────

function lighten(hex: string, amount: number): string {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  const nr = Math.min(255, r + Math.round((255 - r) * amount));
  const ng = Math.min(255, g + Math.round((255 - g) * amount));
  const nb = Math.min(255, b + Math.round((255 - b) * amount));
  return `#${nr.toString(16).padStart(2, '0')}${ng.toString(16).padStart(2, '0')}${nb.toString(16).padStart(2, '0')}`;
}

export function darken(hex: string, amount: number): string {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  const nr = Math.max(0, Math.round(r * (1 - amount)));
  const ng = Math.max(0, Math.round(g * (1 - amount)));
  const nb = Math.max(0, Math.round(b * (1 - amount)));
  return `#${nr.toString(16).padStart(2, '0')}${ng.toString(16).padStart(2, '0')}${nb.toString(16).padStart(2, '0')}`;
}

// ─── Color palettes — professional office style ────────────────────────────

const ACCENT_PALETTES = [
  '#5E81AC', '#81A1C1', '#88C0D0', '#8FBCBB',
  '#A3BE8C', '#B48EAD', '#BF616A', '#D08770',
  '#EBCB8B', '#5B8C5A',
];

const FLOOR_PALETTES = [
  '#3B4252', '#434C5E', '#4C566A', '#3F4A5C',
];

const FURNITURE_PALETTES = [
  '#4C566A', '#5A6578', '#3B4252', '#5E6B7E',
];

// ─── Color generation ───────────────────────────────────────────────────────

export function generateColors(groupId: string): RoomColors {
  const h = Math.abs(hashStr(groupId));
  const accent = ACCENT_PALETTES[h % ACCENT_PALETTES.length];
  const floor = FLOOR_PALETTES[(h >>> 4) % FLOOR_PALETTES.length];
  const furniture = FURNITURE_PALETTES[(h >>> 8) % FURNITURE_PALETTES.length];

  return {
    wall: '#2E3440',
    wallDark: '#252B35',
    accent,
    accentDark: darken(accent, 0.2),
    floor,
    floorDark: darken(floor, 0.15),
    furniture,
    furnitureDark: darken(furniture, 0.2),
    screen: '#88C0D0',
    screenGlow: '#5E81AC',
    trim: '#D8DEE9',
  };
}

// ─── Template selection ────────────────────────────────────────────────────

function makeTemplate(room: OfficeRoom): RoomTemplate {
  const pixelW = room.width * TILE_SIZE;
  const pixelH = room.height * TILE_SIZE;

  const doorX = (room.doorTileX - room.tileX) * TILE_SIZE + TILE_SIZE / 2;
  const doorY = (room.doorTileY - room.tileY) * TILE_SIZE + TILE_SIZE / 2;

  const rows: number[][] = [];
  for (let r = 0; r < pixelH; r++) {
    rows.push(new Array(pixelW).fill(0));
  }

  return {
    width: pixelW,
    height: pixelH,
    pixels: rows,
    doorX: Math.floor(doorX / TILE_SIZE),
    doorY: Math.floor(doorY / TILE_SIZE),
    labelY: -4,
  };
}

// ─── Public API ─────────────────────────────────────────────────────────────

export function generateHouse(
  groupId: string,
  _memberCount: number,
  room?: OfficeRoom,
): GeneratedHouse {
  if (room) {
    return {
      template: makeTemplate(room),
      colors: generateColors(groupId),
    };
  }
  return {
    template: {
      width: 96,
      height: 80,
      pixels: [],
      doorX: 3,
      doorY: 4,
      labelY: -4,
    },
    colors: generateColors(groupId),
  };
}

// ─── Canvas drawing helpers ─────────────────────────────────────────────────

/** Draw a filled rectangle at pixel-perfect coords. */
function px(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, color: string): void {
  ctx.fillStyle = color;
  ctx.fillRect(Math.round(x), Math.round(y), Math.round(w), Math.round(h));
}

// ═══════════════════════════════════════════════════════════════════════════
// Furniture Color Palette — dedicated palette for all interior objects
// ═══════════════════════════════════════════════════════════════════════════

const FURNITURE_COLORS = {
  // Wood Tones
  wood_dark: '#4a3425',
  wood_mid: '#6b4a33',
  wood_light: '#8b5e42',
  wood_highlight: '#a47551',
  // Metal & Stone
  metal_dark: '#475569',
  metal_mid: '#64748b',
  metal_light: '#94a3b8',
  // Fabric & Upholstery
  fabric_green_dark: '#3f6844',
  fabric_green_mid: '#527853',
  fabric_green_light: '#6a996d',
  fabric_blue_dark: '#334155',
  fabric_blue_mid: '#4a5568',
  fabric_blue_light: '#64748b',
  // Electronics
  screen_bg: '#1e293b',
  screen_bezel: '#334155',
  screen_glint: '#e0f2fe',
  // Misc
  plant_pot: '#c2410c',
  plant_leaf_dark: '#15803d',
  plant_leaf_mid: '#22c55e',
  plant_leaf_light: '#4ade80',
  paper: '#f1f5f9',
  rug_dark: '#475569',
  rug_light: '#64748b',
  shadow: 'rgba(46, 52, 64, 0.25)',
};

/** Helper for drawing a simple hard shadow under an object */
function drawObjectShadow(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number) {
  ctx.fillStyle = FURNITURE_COLORS.shadow;
  ctx.fillRect(x, y + h - 2, w, 4);
}


export interface RoomEntity {
  canvas: OffscreenCanvas | HTMLCanvasElement;
  dx: number;
  dy: number;
  width: number;
  height: number;
  depthY: number;
}

export class EntityBuilder {
  entities: RoomEntity[] = [];
  public draw(w: number, h: number, x: number, y: number, depthAnchorY: number, drawFn: (ctx: CanvasRenderingContext2D) => void) {
    const oc = typeof OffscreenCanvas !== 'undefined' ? new OffscreenCanvas(w, h) : document.createElement('canvas');
    if (oc instanceof HTMLCanvasElement) {
      oc.width = w; oc.height = h;
    }
    const octx = oc.getContext('2d') as CanvasRenderingContext2D;
    if (!octx) return;
    (octx as any).imageSmoothingEnabled = false;

    octx.translate(-x, -y);
    drawFn(octx);
    octx.translate(x, y);

    this.entities.push({
      canvas: oc,
      dx: x,
      dy: y,
      width: w,
      height: h,
      depthY: y + depthAnchorY
    });
  }
}

export let activeEntityBuilder: EntityBuilder | null = null;
export function setActiveEntityBuilder(b: EntityBuilder | null) { activeEntityBuilder = b; }

function wrapEntity(w: number, h: number, x: number, y: number, dy: number, drawFn: (ctx: CanvasRenderingContext2D) => void) {
  if (activeEntityBuilder) {
    const builder = activeEntityBuilder;
    activeEntityBuilder = null;
    builder.draw(w, h, x, y, dy, drawFn);
    activeEntityBuilder = builder;
    return true;
  }
  return false;
}

// ═══════════════════════════════════════════════════════════════════════════
// Phase 3 — Gemini Furniture Catalog (exact hex codes)
// ═══════════════════════════════════════════════════════════════════════════

// ---------------------------------------------------------------------------
// Desks & Tables
// ---------------------------------------------------------------------------

/** OFFICE_DESK (2x1 tiles) — Gemini pixel-perfect version */
function drawOfficeDesk(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  if (wrapEntity(32*s, 16*s, x, y, 12*s, (octx) => drawOfficeDesk(octx, x, y, s))) return;
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Shadow
  drawRect(0, 10, 32, 6, 'rgba(0,0,0,0.2)');
  // Legs (Metal/Dark Wood)
  drawRect(2, 4, 3, 10, '#2d1b14');
  drawRect(27, 4, 3, 10, '#2d1b14');
  // Modesty panel
  drawRect(5, 4, 22, 8, '#4a2e1b');
  drawRect(5, 4, 22, 1, '#2d1b14'); // panel shadow
  // Desk Top
  drawRect(0, 0, 32, 6, '#7a4b28'); // base
  drawRect(0, 0, 32, 1, '#9e673b'); // highlight
  drawRect(0, 5, 32, 1, '#4a2e1b'); // bottom edge
  drawRect(0, 0, 1, 6, '#9e673b'); // left edge
  drawRect(31, 0, 1, 6, '#4a2e1b'); // right edge
  // Drawers on right
  drawRect(24, 6, 7, 8, '#5c3820');
  drawRect(25, 7, 5, 3, '#4a2e1b'); // drawer 1
  drawRect(26, 8, 3, 1, '#9e673b'); // handle
  drawRect(25, 11, 5, 3, '#4a2e1b'); // drawer 2
  drawRect(26, 12, 3, 1, '#9e673b'); // handle
}

/** MANAGER_DESK (32x16) — Gemini pixel-perfect version */
function drawManagerDesk(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  if (wrapEntity(48*s, 32*s, x, y, 24*s, (octx) => drawManagerDesk(octx, x, y, s))) return;
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Shadow
  drawRect(0, 20, 48, 12, 'rgba(0,0,0,0.2)');
  // Desk Body
  drawRect(2, 8, 44, 20, '#2d1b14');
  drawRect(4, 10, 10, 16, '#4a2e1b'); // left drawer bank
  drawRect(34, 10, 10, 16, '#4a2e1b'); // right drawer bank
  // Drawers
  [12, 18, 24].forEach(dy => {
    drawRect(5, dy, 8, 4, '#5c3820');
    drawRect(6, dy+1, 6, 1, '#7a4b28'); // highlight
    drawRect(8, dy+2, 2, 1, '#9e673b'); // handle
    drawRect(35, dy, 8, 4, '#5c3820');
    drawRect(36, dy+1, 6, 1, '#7a4b28');
    drawRect(38, dy+2, 2, 1, '#9e673b');
  });
  // Desk Top
  drawRect(0, 0, 48, 10, '#7a4b28');
  drawRect(0, 0, 48, 1, '#9e673b');
  drawRect(0, 9, 48, 1, '#4a2e1b');
  drawRect(0, 0, 1, 10, '#9e673b');
  drawRect(47, 0, 1, 10, '#4a2e1b');
  // Leather Pad
  drawRect(12, 2, 24, 6, '#064e3b');
  drawRect(12, 2, 24, 1, '#047857');
  drawRect(12, 7, 24, 1, '#022c22');
}

/** CONFERENCE_TABLE (4x2 tiles) — Gemini pixel-perfect version */
function drawConferenceTable(ctx: CanvasRenderingContext2D, x: number, y: number, s: number, tilesW: number, tilesH: number): void {
  if (wrapEntity(tilesW*16*s, tilesH*16*s, x, y, tilesH*8*s, (octx) => drawConferenceTable(octx, x, y, s, tilesW, tilesH))) return;
  const w = tilesW * 16;
  const h = tilesH * 16;
  const drawRect = (px: number, py: number, rw: number, rh: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, rw * s, rh * s);
  };
  // Shadow
  drawRect(4, h/2 + 4, w-8, h/2, 'rgba(0,0,0,0.2)');
  // Base/Legs
  drawRect(8, h/2, w-16, h/2 - 2, '#1e293b');
  drawRect(8, h/2, w-16, 1, '#334155');
  // Top
  drawRect(0, 0, w, h/2 + 2, '#4a2e1b');
  drawRect(0, 0, w, 1, '#7a4b28');
  drawRect(0, 0, 1, h/2 + 2, '#7a4b28');
  drawRect(w-1, 0, 1, h/2 + 2, '#2d1b14');
  drawRect(0, h/2 + 1, w, 1, '#2d1b14');
  // Center inlay
  drawRect(4, 2, w-8, h/2 - 2, '#2d1b14');
  drawRect(4, 2, w-8, 1, '#1a0f0b');
}

/** COUNTER (1 tile wide x N tiles): top #94a3b8, body #f1f5f9 */
function drawCounter(ctx: CanvasRenderingContext2D, x: number, y: number, s: number, tilesLen: number): void {
  if (wrapEntity(tilesLen*16*s, 16*s, x, y, 12*s, (octx) => drawCounter(octx, x, y, s, tilesLen))) return;
  const w = tilesLen * 16;
  const drawRect = (px: number, py: number, rw: number, rh: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, rw * s, rh * s);
  };
  // Body
  drawRect(0, 6, w, 10, '#e2e8f0');
  drawRect(0, 15, w, 1, '#94a3b8');
  // Cabinets
  for(let i=0; i<tilesLen; i++) {
    drawRect(i*16 + 2, 8, 12, 6, '#cbd5e1');
    drawRect(i*16 + 2, 8, 12, 1, '#f8fafc');
    drawRect(i*16 + 13, 8, 1, 6, '#94a3b8');
    drawRect(i*16 + 2, 13, 12, 1, '#94a3b8');
    // Handle
    drawRect(i*16 + 6, 10, 4, 1, '#475569');
  }
  // Top
  drawRect(0, 0, w, 6, '#1e293b');
  drawRect(0, 0, w, 1, '#475569');
  drawRect(0, 5, w, 1, '#0f172a');
}

// ---------------------------------------------------------------------------
// Seating
// ---------------------------------------------------------------------------

/** OFFICE_CHAIR (1x1) — Gemini pixel-perfect version */
function drawOfficeChair(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  if (wrapEntity(16*s, 16*s, x, y, 14*s, (octx) => drawOfficeChair(octx, x, y, s))) return;
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Wheels/Base
  drawRect(7, 12, 2, 3, '#1e293b'); // cylinder
  drawRect(3, 14, 10, 1, '#334155'); // legs
  drawRect(7, 13, 2, 2, '#475569'); // center hub
  drawRect(2, 14, 2, 2, '#0f172a'); // wheel L
  drawRect(12, 14, 2, 2, '#0f172a'); // wheel R
  // Seat
  drawRect(3, 9, 10, 3, '#1d4ed8'); // base blue
  drawRect(3, 9, 10, 1, '#3b82f6'); // highlight
  drawRect(3, 11, 10, 1, '#1e3a8a'); // shadow
  // Armrests
  drawRect(2, 7, 2, 4, '#334155');
  drawRect(12, 7, 2, 4, '#334155');
  // Backrest
  drawRect(4, 2, 8, 7, '#1d4ed8');
  drawRect(4, 2, 8, 1, '#3b82f6'); // top highlight
  drawRect(4, 2, 1, 7, '#3b82f6'); // left highlight
  drawRect(11, 3, 1, 6, '#1e3a8a'); // right shadow
}

/** GUEST_CHAIR (1x1): wood #8b5e42 */
function drawGuestChair(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  if (wrapEntity(16*s, 16*s, x, y, 14*s, (octx) => drawGuestChair(octx, x, y, s))) return;
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Legs
  drawRect(3, 10, 2, 6, '#2d1b14');
  drawRect(11, 10, 2, 6, '#2d1b14');
  // Seat
  drawRect(2, 7, 12, 4, '#047857');
  drawRect(2, 7, 12, 1, '#10b981');
  drawRect(2, 10, 12, 1, '#022c22');
  // Backrest
  drawRect(3, 1, 10, 6, '#047857');
  drawRect(3, 1, 10, 1, '#10b981');
  drawRect(3, 1, 1, 6, '#10b981');
  drawRect(12, 2, 1, 5, '#022c22');
  // Wood frame
  drawRect(2, 0, 12, 1, '#4a2e1b');
  drawRect(2, 0, 1, 8, '#4a2e1b');
  drawRect(13, 0, 1, 8, '#4a2e1b');
}

/** LOUNGE_SOFA (2x1) — Gemini pixel-perfect version */
function drawLoungeSofa(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  if (wrapEntity(32*s, 16*s, x, y, 14*s, (octx) => drawLoungeSofa(octx, x, y, s))) return;
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Shadow
  drawRect(2, 14, 28, 2, 'rgba(0,0,0,0.2)');
  // Base
  drawRect(1, 12, 30, 3, '#1e293b');
  // Backrest
  drawRect(2, 2, 28, 8, '#b91c1c');
  drawRect(2, 2, 28, 1, '#ef4444');
  drawRect(2, 9, 28, 1, '#7f1d1d');
  // Seat Cushions
  drawRect(4, 8, 11, 5, '#b91c1c');
  drawRect(4, 8, 11, 1, '#ef4444');
  drawRect(4, 12, 11, 1, '#7f1d1d');
  drawRect(17, 8, 11, 5, '#b91c1c');
  drawRect(17, 8, 11, 1, '#ef4444');
  drawRect(17, 12, 11, 1, '#7f1d1d');
  // Armrests
  drawRect(0, 5, 4, 8, '#991b1b');
  drawRect(0, 5, 4, 1, '#ef4444');
  drawRect(0, 5, 1, 8, '#ef4444');
  drawRect(3, 6, 1, 7, '#7f1d1d');
  drawRect(28, 5, 4, 8, '#991b1b');
  drawRect(28, 5, 4, 1, '#ef4444');
  drawRect(28, 5, 1, 8, '#ef4444');
  drawRect(31, 6, 1, 7, '#7f1d1d');
}

// ---------------------------------------------------------------------------
// Storage & Appliances
// ---------------------------------------------------------------------------

/** FILING_CABINET (1x1): body #d6c7a1, handles #475569 */
function drawFilingCabinet(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  if (wrapEntity(16*s, 16*s, x, y, 14*s, (octx) => drawFilingCabinet(octx, x, y, s))) return;
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Body
  drawRect(2, 1, 12, 14, '#94a3b8');
  drawRect(2, 1, 12, 1, '#cbd5e1');
  drawRect(2, 1, 1, 14, '#cbd5e1');
  drawRect(13, 1, 1, 14, '#475569');
  drawRect(2, 14, 12, 1, '#475569');
  // Drawers
  [3, 7, 11].forEach(dy => {
    drawRect(3, dy, 10, 3, '#64748b');
    drawRect(3, dy, 10, 1, '#94a3b8');
    drawRect(3, dy+2, 10, 1, '#475569');
    // Handle
    drawRect(6, dy+1, 4, 1, '#cbd5e1');
  });
}

/** BOOKSHELF (1x2 tiles tall) — Gemini pixel-perfect version */
function drawBookshelf(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  if (wrapEntity(16*s, 32*s, x, y, 30*s, (octx) => drawBookshelf(octx, x, y, s))) return;
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Shadow
  drawRect(1, 30, 14, 2, 'rgba(0,0,0,0.2)');
  // Frame
  drawRect(0, 0, 16, 32, '#4a2e1b');
  drawRect(0, 0, 16, 1, '#7a4b28'); // top highlight
  drawRect(0, 0, 1, 32, '#7a4b28'); // left highlight
  drawRect(15, 0, 1, 32, '#2d1b14'); // right shadow
  // Backing
  drawRect(2, 2, 12, 28, '#2d1b14');
  // Shelves
  [8, 16, 24].forEach(sy => {
    drawRect(2, sy, 12, 2, '#5c3820');
    drawRect(2, sy, 12, 1, '#7a4b28'); // shelf highlight
  });
  // Books (Row 1)
  drawRect(3, 3, 2, 5, '#b91c1c'); drawRect(3, 3, 1, 5, '#ef4444');
  drawRect(5, 4, 2, 4, '#1d4ed8'); drawRect(5, 4, 1, 4, '#3b82f6');
  drawRect(8, 2, 3, 6, '#047857'); drawRect(8, 2, 1, 6, '#10b981');
  // Books (Row 2)
  drawRect(4, 11, 2, 5, '#d97706');
  drawRect(6, 12, 1, 4, '#f59e0b');
  drawRect(10, 10, 3, 6, '#6d28d9');
  // Books (Row 3)
  drawRect(3, 19, 4, 5, '#be185d');
  drawRect(8, 20, 2, 4, '#0369a1');
  drawRect(11, 18, 2, 6, '#15803d');
}

/** WATER_COOLER (1x1): body #f1f5f9, bottle #a2d2ff */
function drawWaterCooler(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  if (wrapEntity(16*s, 16*s, x, y, 14*s, (octx) => drawWaterCooler(octx, x, y, s))) return;
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Base
  drawRect(3, 8, 10, 8, '#e2e8f0');
  drawRect(3, 8, 2, 8, '#ffffff'); // highlight
  drawRect(11, 8, 2, 8, '#94a3b8'); // shadow
  // Drip tray
  drawRect(4, 12, 8, 2, '#cbd5e1');
  drawRect(5, 12, 6, 1, '#475569'); // grate
  // Taps
  drawRect(5, 9, 2, 2, '#ef4444'); // hot
  drawRect(9, 9, 2, 2, '#3b82f6'); // cold
  // Jug
  drawRect(4, 1, 8, 7, '#bae6fd');
  drawRect(5, 0, 6, 1, '#bae6fd'); // top curve
  drawRect(4, 1, 2, 7, '#e0f2fe'); // glass glint
  drawRect(10, 1, 2, 7, '#7dd3fc'); // glass shadow
  // Water level
  drawRect(4, 3, 8, 5, '#38bdf8');
  drawRect(4, 3, 2, 5, '#7dd3fc'); // water glint
  drawRect(10, 3, 2, 5, '#0284c7'); // water shadow
  // Bubbles
  drawRect(6, 5, 1, 1, '#e0f2fe');
  drawRect(8, 6, 1, 1, '#e0f2fe');
  drawRect(7, 4, 1, 1, '#e0f2fe');
}

/** MICROWAVE (1x1): body #e2e8f0, window #0f172a */
function drawMicrowave(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  if (wrapEntity(16*s, 16*s, x, y, 12*s, (octx) => drawMicrowave(octx, x, y, s))) return;
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Body
  drawRect(1, 4, 14, 9, '#cbd5e1');
  drawRect(1, 4, 14, 1, '#f8fafc'); // highlight
  drawRect(1, 12, 14, 1, '#94a3b8'); // shadow
  // Window
  drawRect(2, 6, 8, 5, '#0f172a');
  drawRect(3, 7, 6, 3, '#1e293b'); // inner glow
  drawRect(7, 6, 2, 5, 'rgba(255,255,255,0.1)'); // glint
  // Control Panel
  drawRect(11, 6, 3, 6, '#0f172a');
  drawRect(12, 7, 1, 1, '#a3e635'); // time display
  drawRect(12, 9, 1, 1, '#ef4444'); // stop button
  drawRect(12, 10, 1, 1, '#f8fafc'); // start button
}

/** MINI_FRIDGE (1x1): body #cbd5e1, handle #475569 */
function drawMiniFridge(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  if (wrapEntity(16*s, 16*s, x, y, 14*s, (octx) => drawMiniFridge(octx, x, y, s))) return;
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Body
  drawRect(2, 2, 12, 14, '#e2e8f0');
  drawRect(2, 2, 12, 1, '#ffffff'); // top highlight
  drawRect(2, 2, 1, 14, '#ffffff'); // left highlight
  drawRect(13, 2, 1, 14, '#94a3b8'); // right shadow
  // Door separation
  drawRect(2, 6, 12, 1, '#94a3b8');
  // Handles
  drawRect(11, 3, 1, 2, '#475569');
  drawRect(11, 8, 1, 4, '#475569');
}

// ---------------------------------------------------------------------------
// Electronics & Clutter
// ---------------------------------------------------------------------------

/** COMPUTER_MONITOR (1x1) — Gemini pixel-perfect version */
function drawComputerMonitor(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  if (wrapEntity(16*s, 16*s, x, y, 12*s, (octx) => drawComputerMonitor(octx, x, y, s))) return;
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Stand
  drawRect(6, 12, 4, 3, '#475569');
  drawRect(4, 14, 8, 2, '#334155');
  // Bezel
  drawRect(1, 2, 14, 10, '#1e293b');
  drawRect(1, 2, 14, 1, '#475569'); // top highlight
  // Screen
  drawRect(2, 3, 12, 8, '#0f172a'); // screen bg
  // Code/Content
  drawRect(3, 4, 4, 1, '#38bdf8');
  drawRect(3, 6, 8, 1, '#a3e635');
  drawRect(3, 8, 6, 1, '#f472b6');
  // Glint
  drawRect(11, 3, 3, 8, 'rgba(255,255,255,0.1)');
  drawRect(12, 3, 2, 8, 'rgba(255,255,255,0.1)');
}

/** KEYBOARD (1x1, placed on desks): light gray #e2e8f0 on dark #475569 */
function drawKeyboard(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  if (wrapEntity(16*s, 16*s, x, y, 8*s, (octx) => drawKeyboard(octx, x, y, s))) return;
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  drawRect(2, 6, 12, 4, '#1e293b');
  drawRect(2, 6, 12, 1, '#334155');
  drawRect(2, 9, 12, 1, '#0f172a');
  // Keys
  drawRect(3, 7, 10, 1, '#cbd5e1');
  drawRect(3, 8, 8, 1, '#cbd5e1');
}

/** PLANT_POT (1x1) — Gemini pixel-perfect version */
function drawPlantPot(ctx: CanvasRenderingContext2D, originalX: number, originalY: number, s: number, ox: number = (Math.random()-0.5)*4*s, oy: number = (Math.random()-0.5)*4*s): void {
  const x = originalX + ox;
  const y = originalY + oy;
  if (wrapEntity(16*s, 16*s, x, y, 14*s, (octx) => drawPlantPot(octx, x, y, s, 0, 0))) return;
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Shadow
  drawRect(3, 14, 10, 2, 'rgba(0,0,0,0.2)');
  // Pot
  drawRect(4, 9, 8, 6, '#a0522d'); // base
  drawRect(3, 8, 10, 2, '#cd853f'); // rim
  drawRect(4, 10, 2, 5, '#cd853f'); // pot highlight
  drawRect(10, 10, 2, 5, '#5c2e16'); // pot shadow
  // Dirt
  drawRect(5, 8, 6, 1, '#3e1f0f');
  // Leaves
  const darkGreen = '#1e5928';
  const midGreen = '#32843d';
  const lightGreen = '#57b75b';
  // Back leaves
  drawRect(4, 3, 3, 3, darkGreen);
  drawRect(9, 2, 4, 4, darkGreen);
  // Mid leaves
  drawRect(3, 5, 5, 4, midGreen);
  drawRect(8, 4, 5, 5, midGreen);
  // Front/Highlight leaves
  drawRect(4, 4, 2, 2, lightGreen);
  drawRect(9, 5, 2, 2, lightGreen);
  drawRect(6, 6, 3, 3, lightGreen);
  // Stem
  drawRect(7, 7, 2, 2, darkGreen);
}

/** MUG / Coffee Mug: pixel-perfect ~6x6 mug with steam */
function drawMug(ctx: CanvasRenderingContext2D, originalX: number, originalY: number, s: number, ox: number = (Math.random()-0.5)*4*s, oy: number = (Math.random()-0.5)*4*s): void {
  const x = originalX + ox;
  const y = originalY + oy;
  if (wrapEntity(16*s, 16*s, x, y, 8*s, (octx) => drawMug(octx, x, y, s, 0, 0))) return;
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  drawRect(5, 6, 6, 6, '#f8fafc');
  drawRect(5, 6, 6, 1, '#ffffff');
  drawRect(5, 11, 6, 1, '#cbd5e1');
  drawRect(10, 7, 1, 4, '#cbd5e1');
  // Coffee
  drawRect(6, 6, 4, 1, '#4a2e1b');
  // Handle
  drawRect(11, 7, 2, 3, '#f8fafc');
  drawRect(11, 8, 1, 1, '#cbd5e1'); // inner hole
  // Steam
  drawRect(6, 3, 1, 2, 'rgba(255,255,255,0.5)');
  drawRect(8, 2, 1, 2, 'rgba(255,255,255,0.5)');
}

/** PAPER_STACK (1x1): white lines #f1f5f9 on shadow #e2e8f0 */
function drawPaperStack(ctx: CanvasRenderingContext2D, originalX: number, originalY: number, s: number, ox: number = (Math.random()-0.5)*4*s, oy: number = (Math.random()-0.5)*4*s): void {
  const x = originalX + ox;
  const y = originalY + oy;
  if (wrapEntity(16*s, 16*s, x, y, 8*s, (octx) => drawPaperStack(octx, x, y, s, 0, 0))) return;
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Shadow
  drawRect(4, 5, 8, 7, 'rgba(0,0,0,0.1)');
  // Papers
  drawRect(3, 4, 8, 6, '#f8fafc');
  drawRect(3, 9, 8, 1, '#cbd5e1');
  drawRect(10, 4, 1, 6, '#cbd5e1');
  // Top paper offset
  drawRect(4, 3, 8, 6, '#ffffff');
  drawRect(4, 8, 8, 1, '#e2e8f0');
  drawRect(11, 3, 1, 6, '#e2e8f0');
  // Lines
  drawRect(5, 4, 5, 1, '#94a3b8');
  drawRect(5, 6, 6, 1, '#94a3b8');
}

// ---------------------------------------------------------------------------
// New Furniture Items (Bible spec additions)
// ---------------------------------------------------------------------------

/** WALL_CLOCK (on back wall): pixel-perfect octagonal clock, 16x16 tile */
function drawWallClock(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Frame
  drawRect(4, 2, 8, 12, '#4a2e1b');
  drawRect(2, 4, 12, 8, '#4a2e1b');
  drawRect(3, 3, 10, 10, '#4a2e1b');
  // Face
  drawRect(5, 3, 6, 10, '#f8fafc');
  drawRect(3, 5, 10, 6, '#f8fafc');
  drawRect(4, 4, 8, 8, '#f8fafc');
  // Shadow on face
  drawRect(4, 10, 8, 2, '#e2e8f0');
  // Hands
  drawRect(7, 7, 2, 2, '#0f172a'); // center
  drawRect(8, 4, 1, 3, '#0f172a'); // minute
  drawRect(8, 7, 3, 1, '#0f172a'); // hour
}

/** WALL_POSTER / Motivational Poster (on walls): pixel-perfect 16x16 tile */
function drawWallPoster(ctx: CanvasRenderingContext2D, x: number, y: number, s: number, bgColor: string): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Frame
  drawRect(2, 1, 12, 14, '#1e293b');
  drawRect(2, 1, 12, 1, '#334155');
  drawRect(2, 14, 12, 1, '#0f172a');
  // Poster
  drawRect(3, 2, 10, 12, bgColor);
  // Graphic
  drawRect(4, 4, 8, 5, '#ffffff');
  drawRect(5, 5, 6, 3, '#fde047');
  // Text
  drawRect(4, 10, 8, 1, '#ffffff');
  drawRect(5, 12, 6, 1, '#ffffff');
}

/** PRINTER_COPIER (1x1): pixel-perfect 16x16 tile */
function drawPrinter(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Body
  drawRect(1, 6, 14, 8, '#e2e8f0');
  drawRect(1, 6, 14, 1, '#ffffff');
  drawRect(1, 13, 14, 1, '#94a3b8');
  // Paper In
  drawRect(3, 2, 10, 4, '#f8fafc');
  drawRect(3, 2, 10, 1, '#cbd5e1');
  // Paper Out
  drawRect(4, 14, 8, 2, '#f8fafc');
  drawRect(4, 14, 8, 1, '#cbd5e1');
  // Output slot
  drawRect(3, 11, 10, 2, '#1e293b');
  // Control Panel
  drawRect(11, 7, 3, 3, '#334155');
  drawRect(12, 8, 1, 1, '#22c55e'); // LED
}

/** VENDING_MACHINE (16x32): pixel-perfect tall vending machine */
function drawVendingMachine(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Body
  drawRect(0, 0, 16, 32, '#b91c1c');
  drawRect(0, 0, 16, 1, '#ef4444'); // highlight
  drawRect(0, 0, 1, 32, '#ef4444');
  drawRect(15, 0, 1, 32, '#7f1d1d'); // shadow
  // Glass
  drawRect(2, 3, 10, 18, '#0f172a');
  drawRect(2, 3, 10, 1, '#1e293b');
  // Snacks
  const colors = ['#fde047', '#3b82f6', '#22c55e', '#f97316', '#ec4899'];
  for(let row=0; row<4; row++) {
    drawRect(2, 6 + row*4, 10, 1, '#334155'); // shelf
    for(let col=0; col<3; col++) {
       drawRect(3 + col*3, 4 + row*4, 2, 2, colors[(row+col)%colors.length]);
    }
  }
  // Glass Glint
  drawRect(3, 3, 2, 18, 'rgba(255,255,255,0.15)');
  drawRect(6, 3, 1, 18, 'rgba(255,255,255,0.1)');
  // Control Panel
  drawRect(13, 3, 2, 18, '#1e293b');
  drawRect(13, 5, 2, 2, '#ef4444'); // display
  drawRect(13, 8, 2, 4, '#475569'); // buttons
  drawRect(13, 14, 2, 3, '#0f172a'); // coin slot
  // Dispenser
  drawRect(2, 24, 12, 5, '#0f172a');
  drawRect(2, 24, 12, 1, '#1e293b'); // flap
}

/** STICKY_NOTES: pixel-perfect 16x16 cluster of sticky notes on a wall */
function drawStickyNotes(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Note 1
  drawRect(3, 3, 5, 5, '#fde047');
  drawRect(3, 7, 5, 1, '#ca8a04');
  drawRect(4, 4, 3, 1, '#713f12');
  // Note 2
  drawRect(9, 5, 5, 5, '#86efac');
  drawRect(9, 9, 5, 1, '#16a34a');
  drawRect(10, 6, 3, 1, '#14532d');
  // Note 3
  drawRect(5, 9, 5, 5, '#93c5fd');
  drawRect(5, 13, 5, 1, '#2563eb');
  drawRect(6, 10, 3, 1, '#1e3a8a');
}

/** STALL_PARTITION / Bathroom Toilet Stall: pixel-perfect 16x16 tile (top-down view) */
function drawStallPartition(ctx: CanvasRenderingContext2D, x: number, y: number, s: number, height: number): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  const hTiles = Math.floor(height / 16);
  const pixelH = hTiles * 16;
  // Back wall
  drawRect(0, 0, 16, 2, '#94a3b8');
  // Side walls
  drawRect(0, 0, 2, pixelH, '#cbd5e1');
  drawRect(14, 0, 2, pixelH, '#cbd5e1');
  // Door
  drawRect(2, 2, 12, pixelH - 4, '#e2e8f0');
  drawRect(2, 2, 12, 1, '#ffffff'); // door highlight
  drawRect(13, 2, 1, pixelH - 4, '#94a3b8'); // door shadow
  // Handle
  drawRect(11, pixelH/2 - 1, 2, 4, '#475569');
  // Feet
  drawRect(0, pixelH, 2, 2, '#475569');
  drawRect(14, pixelH, 2, 2, '#475569');
}

/** BATHROOM_SINK: pixel-perfect 16x16 sink with mirror */
function drawBathroomSink(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Counter
  drawRect(0, 8, 16, 8, '#cbd5e1');
  drawRect(0, 8, 16, 1, '#f8fafc'); // highlight
  drawRect(0, 15, 16, 1, '#94a3b8'); // shadow
  // Sink Basin
  drawRect(3, 10, 10, 5, '#ffffff');
  drawRect(4, 11, 8, 3, '#e2e8f0'); // inner shadow
  drawRect(7, 12, 2, 1, '#0f172a'); // drain
  // Faucet
  drawRect(7, 7, 2, 3, '#94a3b8');
  drawRect(7, 7, 3, 1, '#e2e8f0'); // spout
  drawRect(6, 8, 1, 1, '#ef4444'); // hot
  drawRect(9, 8, 1, 1, '#3b82f6'); // cold
}

/** BATHROOM_MIRROR: #e0f2fe with white glint */
function drawBathroomMirror(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Frame
  drawRect(2, 1, 12, 14, '#94a3b8');
  drawRect(2, 1, 12, 1, '#cbd5e1'); // top highlight
  drawRect(2, 14, 12, 1, '#475569'); // bottom shadow
  // Glass
  drawRect(3, 2, 10, 12, '#bae6fd');
  // Glint
  drawRect(4, 3, 2, 10, '#e0f2fe');
  drawRect(7, 3, 1, 10, '#e0f2fe');
}

/** HAND_DRYER: #94a3b8 box on wall */
function drawHandDryer(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Body
  drawRect(4, 4, 8, 10, '#e2e8f0');
  drawRect(4, 4, 8, 1, '#ffffff'); // highlight
  drawRect(4, 13, 8, 1, '#94a3b8'); // shadow
  // Vent
  drawRect(5, 14, 6, 2, '#1e293b');
  // Button/Sensor
  drawRect(7, 10, 2, 2, '#0f172a');
}

/** FRAMED_DIPLOMA: pixel-perfect 16x16 framed diploma/certificate */
function drawFramedDiploma(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Frame
  drawRect(2, 3, 12, 10, '#4a2e1b');
  drawRect(2, 3, 12, 1, '#7a4b28');
  drawRect(2, 12, 12, 1, '#2d1b14');
  // Paper
  drawRect(3, 4, 10, 8, '#f8fafc');
  // Text
  drawRect(5, 5, 6, 1, '#0f172a');
  drawRect(4, 7, 8, 1, '#475569');
  drawRect(5, 9, 6, 1, '#475569');
  // Seal
  drawRect(10, 9, 2, 2, '#b91c1c');
  drawRect(10, 9, 1, 1, '#ef4444'); // seal highlight
}

/** BULLETIN_BOARD: pixel-perfect 16x16 cork bulletin board */
function drawBulletinBoard(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Frame
  drawRect(0, 0, 16, 16, '#7a4b28');
  drawRect(0, 0, 16, 1, '#9e673b');
  drawRect(0, 15, 16, 1, '#4a2e1b');
  // Cork
  drawRect(1, 1, 14, 14, '#d97706');
  drawRect(1, 1, 14, 1, '#f59e0b'); // inner highlight
  drawRect(1, 14, 14, 1, '#b45309'); // inner shadow
  // Notes
  drawRect(3, 3, 4, 5, '#f8fafc'); drawRect(4, 3, 1, 1, '#ef4444'); // white note, red pin
  drawRect(9, 4, 5, 4, '#fde047'); drawRect(11, 4, 1, 1, '#3b82f6'); // yellow note, blue pin
  drawRect(5, 9, 4, 4, '#bbf7d0'); drawRect(6, 9, 1, 1, '#ef4444'); // green note, red pin
  drawRect(10, 10, 3, 3, '#fbcfe8'); drawRect(11, 10, 1, 1, '#eab308'); // pink note, yellow pin
}

/** Compact pixels sampled from the approved three-stroke Signal Chorus mark. */
export const COMPANY_SIGN_SIGNAL_STROKES: ReadonlyArray<ReadonlyArray<readonly [number, number]>> = [
  [[12, 4], [10, 3], [7, 3], [5, 4], [4, 6], [4, 9], [5, 11], [7, 12], [10, 12], [12, 11]],
  [[12, 6], [10, 5], [8, 5], [7, 6], [6, 8], [6, 9], [7, 10], [8, 11], [10, 11], [12, 10]],
  [[12, 8], [10, 7], [9, 7], [8, 8], [8, 9], [9, 10], [10, 10], [12, 9]],
];
export const COMPANY_SIGN_SIGNAL_COLORS = ['#102a6b', '#14b8a6', '#38bdf8'] as const;

/** COMPANY_SIGN: pixel-perfect 32x16 Signal Chorus company logo sign */
function drawCompanySign(ctx: CanvasRenderingContext2D, x: number, y: number, s: number, accent: string): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Backing
  drawRect(0, 2, 32, 12, '#1e293b');
  drawRect(0, 2, 32, 1, '#334155');
  drawRect(0, 13, 32, 1, '#0f172a');
  // Signal Chorus: three nested, open signal strokes in the approved palette.
  for (const [stroke, color] of [
    [COMPANY_SIGN_SIGNAL_STROKES[0], COMPANY_SIGN_SIGNAL_COLORS[0]],
    [COMPANY_SIGN_SIGNAL_STROKES[1], COMPANY_SIGN_SIGNAL_COLORS[1]],
    [COMPANY_SIGN_SIGNAL_STROKES[2], COMPANY_SIGN_SIGNAL_COLORS[2]],
  ] as const) {
    for (const [px, py] of stroke) drawRect(px, py, 1, 1, color);
  }
}

/** MENU_BOARD: chalkboard style for kitchen */
function drawMenuBoard(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Frame
  drawRect(1, 1, 14, 14, '#4a2e1b');
  drawRect(1, 1, 14, 1, '#7a4b28');
  drawRect(1, 14, 14, 1, '#2d1b14');
  // Board
  drawRect(2, 2, 12, 12, '#1e293b');
  // Chalk
  drawRect(3, 4, 10, 1, '#f8fafc'); // title
  drawRect(3, 7, 8, 1, '#fde047'); // item 1
  drawRect(3, 9, 6, 1, '#fde047'); // item 2
  drawRect(3, 11, 9, 1, '#fde047'); // item 3
}

/** ART_PRINT: decorative framed art for lounge */
function drawArtPrint(ctx: CanvasRenderingContext2D, x: number, y: number, s: number, color: string): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Frame
  drawRect(2, 2, 12, 12, '#0f172a');
  drawRect(2, 2, 12, 1, '#334155');
  // Canvas
  drawRect(3, 3, 10, 10, '#f8fafc');
  // Art
  drawRect(4, 4, 8, 8, color);
  drawRect(5, 5, 4, 4, '#ffffff');
  drawRect(8, 8, 3, 3, '#1e293b');
}

/** EXECUTIVE_CHAIR: nicer office chair for manager */
function drawExecutiveChair(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Base
  drawRect(6, 12, 4, 3, '#1e293b');
  drawRect(2, 14, 12, 1, '#334155');
  drawRect(6, 13, 4, 2, '#475569');
  drawRect(1, 14, 2, 2, '#0f172a');
  drawRect(13, 14, 2, 2, '#0f172a');
  // Seat
  drawRect(2, 9, 12, 3, '#4a2e1b');
  drawRect(2, 9, 12, 1, '#7a4b28');
  drawRect(2, 11, 12, 1, '#2d1b14');
  // Armrests
  drawRect(1, 6, 2, 5, '#1e293b');
  drawRect(13, 6, 2, 5, '#1e293b');
  // Tall Backrest
  drawRect(3, 0, 10, 9, '#4a2e1b');
  drawRect(3, 0, 10, 1, '#7a4b28');
  drawRect(3, 0, 1, 9, '#7a4b28');
  drawRect(12, 1, 1, 8, '#2d1b14');
  // Tufting (buttons)
  drawRect(5, 3, 1, 1, '#2d1b14'); drawRect(10, 3, 1, 1, '#2d1b14');
  drawRect(7, 5, 2, 1, '#2d1b14');
  drawRect(5, 7, 1, 1, '#2d1b14'); drawRect(10, 7, 1, 1, '#2d1b14');
}

/** WASTE_BIN (1x1): gray #94a3b8 */
function drawWasteBin(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Shadow
  drawRect(4, 14, 8, 2, 'rgba(0,0,0,0.2)');
  // Bin
  drawRect(4, 6, 8, 8, '#475569');
  drawRect(3, 5, 10, 2, '#64748b'); // rim
  drawRect(4, 6, 2, 8, '#94a3b8'); // highlight
  drawRect(10, 6, 2, 8, '#1e293b'); // shadow
  // Trash inside
  drawRect(5, 4, 3, 2, '#f8fafc');
  drawRect(8, 3, 2, 2, '#e2e8f0');
  drawRect(6, 3, 2, 1, '#cbd5e1');
}

/** RUG (variable size) — Gemini pixel-perfect version */
function drawRug(ctx: CanvasRenderingContext2D, x: number, y: number, s: number, tilesW: number, tilesH: number): void {
  const w = tilesW * 16;
  const h = tilesH * 16;
  const drawRect = (px: number, py: number, rw: number, rh: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, rw * s, rh * s);
  };
  // Faux elevation shadow (Diorama style)
  ctx.fillStyle = 'rgba(0,0,0,0.2)';
  ctx.fillRect(x - s, y + h*s, w*s + s*2, s * 2);
  ctx.fillRect(x + w*s, y, s * 2, h*s);
  
  // Base
  drawRect(0, 0, w, h, '#7c2d12'); // dark red
  // Fringe (top/bottom)
  for(let i=1; i<w; i+=2) {
    drawRect(i, 0, 1, 1, '#fde047');
    drawRect(i, h-1, 1, 1, '#fde047');
  }
  // Border
  drawRect(1, 1, w-2, h-2, '#b45309');
  drawRect(2, 2, w-4, h-4, '#7c2d12');
  // Pattern
  for(let py=4; py<h-4; py+=4) {
    for(let px=4; px<w-4; px+=4) {
      drawRect(px, py, 2, 2, '#f59e0b');
      drawRect(px+1, py+1, 1, 1, '#fef3c7');
    }
  }
}

// ---------------------------------------------------------------------------
// Additional furniture pieces (conference, server, reception)
// ---------------------------------------------------------------------------

/** Whiteboard for conference rooms */
function drawWhiteboard(ctx: CanvasRenderingContext2D, x: number, y: number, s: number, colors: RoomColors): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  const w = 48; const h = 32;
  // Frame
  drawRect(0, 0, w, h, '#94a3b8');
  drawRect(0, 0, w, 1, '#cbd5e1');
  drawRect(0, h-1, w, 1, '#475569');
  drawRect(0, 0, 1, h, '#cbd5e1');
  drawRect(w-1, 0, 1, h, '#475569');
  // Board
  drawRect(1, 1, w-2, h-2, '#f8fafc');
  // Tray
  drawRect(1, h-2, w-2, 2, '#64748b');
  // Markers
  drawRect(4, h-2, 2, 1, '#ef4444');
  drawRect(7, h-2, 2, 1, '#3b82f6');
  drawRect(10, h-2, 2, 1, '#22c55e');
  // Drawings
  drawRect(4, 4, 8, 1, '#3b82f6');
  drawRect(4, 6, 6, 1, '#3b82f6');
  drawRect(16, 8, 10, 8, '#ef4444');
  drawRect(17, 9, 8, 6, '#f8fafc'); // hollow box
  drawRect(28, 12, 6, 1, '#22c55e');
  drawRect(32, 10, 1, 5, '#22c55e');
}

/** Server rack for server rooms */
function drawServerRack(ctx: CanvasRenderingContext2D, x: number, y: number, s: number, blinkPhase: number): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Shadow
  drawRect(1, 30, 14, 2, 'rgba(0,0,0,0.2)');
  // Frame
  drawRect(0, 0, 16, 32, '#0f172a');
  drawRect(0, 0, 16, 1, '#334155');
  drawRect(0, 0, 1, 32, '#334155');
  drawRect(15, 0, 1, 32, '#000000');
  // Servers
  for(let i=0; i<6; i++) {
    const sy = 2 + i*5;
    drawRect(2, sy, 12, 4, '#1e293b');
    drawRect(2, sy, 12, 1, '#475569'); // highlight
    // Vents
    drawRect(3, sy+1, 4, 2, '#0f172a');
    // LEDs
    const led1 = (blinkPhase + i) % 2 === 0 ? '#22c55e' : '#064e3b';
    const led2 = (blinkPhase + i) % 3 === 0 ? '#3b82f6' : '#1e3a8a';
    const led3 = (blinkPhase + i) % 5 === 0 ? '#ef4444' : '#7f1d1d';
    drawRect(9, sy+1, 1, 1, led1);
    drawRect(11, sy+1, 1, 1, led2);
    drawRect(13, sy+1, 1, 1, led3);
  }
}

/** Coffee machine for kitchens */
function drawCoffeeMachine(ctx: CanvasRenderingContext2D, x: number, y: number, s: number): void {
  const drawRect = (px: number, py: number, w: number, h: number, c: string) => {
    ctx.fillStyle = c; ctx.fillRect(x + px * s, y + py * s, w * s, h * s);
  };
  // Body
  drawRect(2, 2, 12, 14, '#1e293b');
  drawRect(2, 2, 12, 1, '#475569');
  drawRect(2, 15, 12, 1, '#0f172a');
  // Control panel
  drawRect(4, 4, 8, 3, '#334155');
  drawRect(5, 5, 2, 1, '#a3e635'); // display
  drawRect(9, 5, 2, 1, '#f8fafc'); // button
  // Nozzle area
  drawRect(5, 8, 6, 2, '#0f172a');
  drawRect(6, 8, 4, 1, '#334155'); // nozzle
  // Drip tray
  drawRect(4, 13, 8, 2, '#334155');
  drawRect(5, 13, 6, 1, '#475569');
  // Cup
  drawRect(6, 10, 4, 3, '#f8fafc');
  drawRect(6, 10, 4, 1, '#ffffff');
  drawRect(6, 12, 4, 1, '#cbd5e1');
}

/** Reception desk — large L-shaped desk */
function drawReceptionDesk(ctx: CanvasRenderingContext2D, x: number, y: number, s: number, colors: RoomColors): void {
  // Desk front panel (3 tiles wide)
  px(ctx, x, y, s * 20, s * 6, colors.furniture);
  // Top surface
  px(ctx, x - s * 1, y - s * 1, s * 22, s * 2, lighten(colors.furniture, 0.15));
  // Front panel accent
  px(ctx, x + s * 2, y + s * 1, s * 16, s * 4, colors.furnitureDark);
  // Company logo area
  px(ctx, x + s * 7, y + s * 2, s * 6, s * 2, colors.accent);
}

// ═══════════════════════════════════════════════════════════════════════════
// Phase 3 — Room Interior Layout Functions (Stardew-level density)
// ═══════════════════════════════════════════════════════════════════════════

function drawConferenceInterior(
  ctx: CanvasRenderingContext2D,
  room: OfficeRoom,
  x: number, y: number,
  scale: number,
  colors: RoomColors,
): void {
  const s = scale;
  const ts = TILE_SIZE * s;
  const innerX = x + ts;
  const innerY = y + ts;
  const innerTilesW = room.width - 2;
  const innerTilesH = room.height - 2;
  const innerW = innerTilesW * ts;
  const innerH = innerTilesH * ts;

  // ── Step 3: Rug under conference table area ──
  const rugW = Math.min(innerTilesW - 1, 5);
  const rugH = Math.min(innerTilesH - 1, 4);
  const rugX = innerX + (innerW - rugW * ts) / 2;
  const rugY = innerY + (innerH - rugH * ts) / 2;
  drawRug(ctx, rugX, rugY, s, rugW, rugH);

  // ── Step 1: Anchor — Conference table centered ──
  const tableW = Math.min(4, innerTilesW - 2);
  const tableH = Math.min(2, innerTilesH - 2);
  const tablePx = innerX + (innerW - tableW * ts) / 2;
  const tablePy = innerY + (innerH - tableH * ts) / 2;
  drawConferenceTable(ctx, tablePx, tablePy, s, tableW, tableH);

  // ── Step 2: Chairs around table ──
  const chairSpacing = ts;
  // Top row chairs
  const topChairs = Math.max(1, tableW);
  for (let i = 0; i < topChairs; i++) {
    const cx = tablePx + i * chairSpacing + (chairSpacing - ts) / 2;
    const cy = tablePy - ts * 1.1;
    if (cy >= innerY) drawOfficeChair(ctx, cx, cy, s);
  }
  // Bottom row chairs
  for (let i = 0; i < topChairs; i++) {
    const cx = tablePx + i * chairSpacing + (chairSpacing - ts) / 2;
    const cy = tablePy + tableH * ts + s * 2;
    if (cy + ts <= innerY + innerH) drawOfficeChair(ctx, cx, cy, s);
  }

  // ── Step 2: Whiteboard on back wall ──
  const wbX = innerX + (innerW - s * 12) / 2;
  if (wbX >= innerX) drawWhiteboard(ctx, wbX, innerY + s * 1, s, colors);

  // ── Step 4: Clutter pass ──
  // Plant in corner
  drawPlantPot(ctx, innerX + s * 1, innerY + innerH - ts, s);
  // Waste bin opposite corner
  drawWasteBin(ctx, innerX + innerW - ts, innerY + innerH - ts, s);
  // Paper stack on table
  drawPaperStack(ctx, tablePx + ts * 0.5, tablePy + s * 1, s);
  // Mug on table
  drawMug(ctx, tablePx + tableW * ts - ts, tablePy + s * 1, s);

  // ── Wall decorations: company logo poster + wall clock ──
  drawWallPoster(ctx, innerX + s * 1, innerY + s * 0.5, s, '#5E81AC');
  drawWallClock(ctx, innerX + innerW - s * 8, innerY + s * 0.5, s);
}

function drawWorkspaceInterior(
  ctx: CanvasRenderingContext2D,
  room: OfficeRoom,
  x: number, y: number,
  scale: number,
  colors: RoomColors,
): void {
  const s = scale;
  const ts = TILE_SIZE * s;
  const innerX = x + ts;
  const innerY = y + ts;
  const innerTilesW = room.width - 2;
  const innerTilesH = room.height - 2;
  const innerW = innerTilesW * ts;
  const innerH = innerTilesH * ts;
  const roomHash = hashStr(room.id);

  // ── Step 3: Rug (optional, based on hash — Gemini improvement) ──
  if (roomHash % 3 !== 0) { // 2/3 of workspaces get a rug
    const rugW = Math.min(innerTilesW - 1, 4 + (roomHash % 3));
    const rugH = Math.min(innerTilesH - 1, 3 + (roomHash % 2));
    const rugX = innerX + (innerW - rugW * ts) / 2 + (roomHash % 5 - 2) * s * 2; // slight offset
    const rugY = innerY + ts * 0.5;
    drawRug(ctx, rugX, rugY, s, rugW, rugH);
  }


  // ── Step 0: Soft North Boundary Divider ──
  // Replaces missing wall with organic props
  for (let i = 0; i < innerTilesW; i += 2) {
    if (i === 4) continue; // leave an entrance gap
    if (roomHash % 2 === 0) drawPlantPot(ctx, innerX + i * ts, innerY - ts*0.2, s);
    else drawFilingCabinet(ctx, innerX + i * ts, innerY - ts * 0.2, s);
  }

  // ── Step 1: Anchor — Desks against back wall ──
  const deskSpacingX = ts * 2.5;
  const desksPerRow = Math.max(1, Math.floor(innerW / deskSpacingX));
  const numRows = Math.max(1, Math.floor(innerTilesH / 3));

  for (let row = 0; row < numRows; row++) {
    for (let col = 0; col < desksPerRow; col++) {
      const deskHash = hashStr(`${room.id}:${row}:${col}`);
      const deskX = innerX + col * deskSpacingX + s * 2 + (deskHash % 7 - 3) * s; // Random offset
      const deskY = innerY + row * ts * 2.5 + ts * 0.5;

      if (deskX + ts * 2 > innerX + innerW) continue;

      drawOfficeDesk(ctx, deskX, deskY, s);

      // ── Step 4: On every desk: monitor + keyboard + personalized clutter ──
      drawComputerMonitor(ctx, deskX + ts * 0.3, deskY - ts * 0.75, s);
      drawKeyboard(ctx, deskX + ts * 0.3, deskY + ts * 0.05, s);

      // Personalized clutter based on hash (Gemini improvement)
      switch (deskHash % 4) {
        case 0: drawMug(ctx, deskX + ts * 1.4, deskY - ts * 0.1, s); break;
        case 1: drawPaperStack(ctx, deskX + ts * 1.4, deskY - ts * 0.1, s); break;
        case 2: { // Mug AND paper for the messy desk
          drawMug(ctx, deskX + ts * 1.4, deskY - ts * 0.1, s);
          drawPaperStack(ctx, deskX - ts * 0.1, deskY - ts * 0.1, s);
          break;
        }
        // case 3: a clean desk
      }

      // ── Step 2: Chair in front of desk ──
      const chairY = deskY + ts * 1.1;
      if (chairY + ts <= innerY + innerH) {
        drawOfficeChair(ctx, deskX + ts * 0.5, chairY, s);
      }
    }
  }

  // ── Step 2: Storage (varied based on hash — Gemini improvement) ──
  if (roomHash % 4 === 0) {
    drawBookshelf(ctx, innerX + innerW - ts, innerY + s * 2, s);
  } else {
    drawFilingCabinet(ctx, innerX + innerW - ts, innerY + s * 2, s);
    if (innerTilesH > 4) {
      drawFilingCabinet(ctx, innerX + innerW - ts, innerY + ts + s * 4, s);
    }
  }

  // ── Step 4: Clutter — more varied plants and bins (Gemini improvement) ──
  if (roomHash % 5 !== 1) drawPlantPot(ctx, innerX, innerY + innerH - ts, s);
  if (roomHash % 5 > 2) drawWasteBin(ctx, innerX + innerW - ts, innerY + innerH - ts, s);

  // ── Wall decorations: varied based on hash (Gemini improvement) ──
  if (roomHash % 3 === 0) drawBulletinBoard(ctx, innerX + s * 1, innerY + s * 0.5, s);
  else drawWallPoster(ctx, innerX + s * 1, innerY + s * 0.5, s, '#1d4ed8');

  drawWallClock(ctx, innerX + innerW - s * 8, innerY + s * 0.5, s);
  if (innerTilesW > 8) {
    drawStickyNotes(ctx, innerX + s * 8, innerY + s * 0.5, s);
  }
}

function drawLoungeInterior(
  ctx: CanvasRenderingContext2D,
  room: OfficeRoom,
  x: number, y: number,
  scale: number,
  colors: RoomColors,
): void {
  const s = scale;
  const ts = TILE_SIZE * s;
  const innerX = x + ts;
  const innerY = y + ts;
  const innerTilesW = room.width - 2;
  const innerTilesH = room.height - 2;
  const innerW = innerTilesW * ts;
  const innerH = innerTilesH * ts;

  // ── Step 3: Large rug in center ──
  const rugW = Math.min(innerTilesW - 1, 3);
  const rugH = Math.min(innerTilesH - 1, 2);
  drawRug(ctx, innerX + (innerW - rugW * ts) / 2, innerY + (innerH - rugH * ts) / 2, s, rugW, rugH);


  // ── Step 0: Soft North Boundary Divider (Bookshelves) ──
  for (let i = 0; i < innerTilesW - 1; i += 3) {
    if (i !== 3) drawBookshelf(ctx, innerX + i * ts, innerY - ts * 0.5, s);
  }

  // ── Step 1: Anchor — Sofa against back wall ──
  drawLoungeSofa(ctx, innerX + s * 2, innerY + s * 2, s);

  // ── Step 2: Coffee table in front of sofa ──
  // Small table (using conference table at small size)
  const coffeeTableX = innerX + ts;
  const coffeeTableY = innerY + ts * 1.5;
  const ctW = Math.min(2, innerTilesW - 2);
  // Draw a simple coffee table
  px(ctx, coffeeTableX, coffeeTableY, ctW * ts, ts * 0.5, colors.furniture);
  px(ctx, coffeeTableX, coffeeTableY, ctW * ts, s * 1, lighten(colors.furniture, 0.12));
  // Legs
  px(ctx, coffeeTableX + s * 1, coffeeTableY + ts * 0.5, s * 1, s * 2, colors.furnitureDark);
  px(ctx, coffeeTableX + ctW * ts - s * 2, coffeeTableY + ts * 0.5, s * 1, s * 2, colors.furnitureDark);

  // ── Step 4: Mug on coffee table ──
  drawMug(ctx, coffeeTableX + ts * 0.3, coffeeTableY - ts * 0.15, s);

  // ── Step 2: Guest chairs ──
  if (innerTilesH > 3) {
    drawGuestChair(ctx, innerX + innerW - ts * 1.2, innerY + ts * 1.5, s);
  }

  // ── Step 2: Bookshelf on side wall ──
  if (innerTilesH >= 3) {
    drawBookshelf(ctx, innerX + innerW - ts, innerY + s * 1, s);
  }

  // ── Step 4: Clutter — plants in corners, magazine/paper on table ──
  drawPlantPot(ctx, innerX + s * 1, innerY + innerH - ts, s);
  drawPaperStack(ctx, coffeeTableX + ctW * ts - ts * 0.8, coffeeTableY - ts * 0.1, s);
  // Waste bin near door area
  drawWasteBin(ctx, innerX + innerW - ts, innerY + innerH - ts, s);

  // ── Wall decorations: 2 art prints + clock ──
  drawArtPrint(ctx, innerX + s * 1, innerY + s * 0.5, s, '#B48EAD');
  drawArtPrint(ctx, innerX + s * 7, innerY + s * 0.5, s, '#88C0D0');
  drawWallClock(ctx, innerX + innerW - s * 8, innerY + s * 0.5, s);
}

function drawServerInterior(
  ctx: CanvasRenderingContext2D,
  room: OfficeRoom,
  x: number, y: number,
  scale: number,
  _colors: RoomColors,
): void {
  const s = scale;
  const ts = TILE_SIZE * s;
  const innerX = x + ts;
  const innerY = y + ts;
  const innerTilesH = room.height - 2;
  const innerTilesW = room.width - 2;
  const innerW = innerTilesW * ts;
  const innerH = innerTilesH * ts;

  // ── Step 1: Anchor — Server racks along left wall ──
  const rackCount = Math.max(1, Math.floor(innerH / (s * 14)));
  for (let i = 0; i < rackCount; i++) {
    drawServerRack(ctx, innerX + s * 2, innerY + i * s * 14 + s * 1, s, i);
  }

  // ── Step 2: Additional rack on right wall if room is wide enough ──
  if (innerTilesW >= 3) {
    drawServerRack(ctx, innerX + innerW - s * 8, innerY + s * 1, s, 2);
  }

  // ── Step 4: Clutter — small monitor + keyboard workstation ──
  if (innerTilesW >= 3 && innerTilesH >= 3) {
    // Small desk (counter-like)
    const deskX = innerX + (innerW - ts) / 2;
    const deskY = innerY + innerH - ts * 1.5;
    px(ctx, deskX, deskY, ts, ts * 0.5, '#a47551');
    px(ctx, deskX + s * 1, deskY + ts * 0.5, s * 2, s * 2, '#54382d');
    drawComputerMonitor(ctx, deskX + ts * 0.1, deskY - ts * 0.7, s);
    drawKeyboard(ctx, deskX + ts * 0.1, deskY + s * 1, s);
  }

  // Waste bin in corner
  drawWasteBin(ctx, innerX + innerW - ts, innerY + innerH - ts, s);
}

function drawKitchenInterior(
  ctx: CanvasRenderingContext2D,
  room: OfficeRoom,
  x: number, y: number,
  scale: number,
  _colors: RoomColors,
): void {
  const s = scale;
  const ts = TILE_SIZE * s;
  const innerX = x + ts;
  const innerY = y + ts;
  const innerTilesW = room.width - 2;
  const innerTilesH = room.height - 2;
  const innerW = innerTilesW * ts;
  const innerH = innerTilesH * ts;

  // ── Step 1: Anchor — Counter along back wall ──
  const counterLen = Math.min(innerTilesW, 4);
  drawCounter(ctx, innerX + s * 1, innerY + s * 1, s, counterLen);

  // ── Step 2: Microwave on counter ──
  drawMicrowave(ctx, innerX + ts * 0.5, innerY + s * 1 - ts * 0.55, s);

  // ── Step 4: Mug on counter ──
  drawMug(ctx, innerX + ts * 2, innerY + s * 1 - ts * 0.3, s);

  // ── Step 2: Mini fridge next to counter ──
  drawMiniFridge(ctx, innerX + innerW - ts, innerY + s * 1, s);

  // ── Step 2: Water cooler next to fridge ──
  if (innerTilesW >= 4) {
    drawWaterCooler(ctx, innerX + innerW - ts * 2, innerY + s * 1, s);
  }

  // ── Step 2: Coffee machine on counter or separate ──
  if (innerTilesW >= 3) {
    drawCoffeeMachine(ctx, innerX + ts * counterLen - s * 2, innerY + ts * 0.5, s);
  }

  // ── Step 3: Rug in center eating area ──
  if (innerTilesH >= 3) {
    const rugW = Math.min(innerTilesW - 1, 3);
    drawRug(ctx, innerX + ts * 0.5, innerY + ts * 1.5, s, rugW, 1);
  }

  // ── Step 2: Small table for eating (if room is large enough) ──
  if (innerTilesH >= 4) {
    const tableX = innerX + (innerW - ts * 2) / 2;
    const tableY = innerY + ts * 2;
    // Simple eating table
    px(ctx, tableX, tableY, ts * 2, ts * 0.6, '#a47551');
    px(ctx, tableX, tableY, ts * 2, s * 1, lighten('#a47551', 0.1));
    px(ctx, tableX + s * 2, tableY + ts * 0.6, s * 1.5, s * 2, '#54382d');
    px(ctx, tableX + ts * 2 - s * 3.5, tableY + ts * 0.6, s * 1.5, s * 2, '#54382d');
    // Chairs at table
    drawGuestChair(ctx, tableX, tableY + ts * 0.8, s);
    drawGuestChair(ctx, tableX + ts, tableY + ts * 0.8, s);
    // Mug on eating table
    drawMug(ctx, tableX + ts * 0.3, tableY - ts * 0.15, s);
  }

  // ── Step 4: Clutter — plant in corner, waste bin ──
  drawPlantPot(ctx, innerX, innerY + innerH - ts, s);
  drawWasteBin(ctx, innerX + innerW - ts, innerY + innerH - ts, s);

  // ── Wall decorations: menu board + poster ──
  drawMenuBoard(ctx, innerX + innerW - s * 8, innerY + s * 0.5, s);
  drawWallPoster(ctx, innerX + s * 1, innerY + s * 0.5, s, '#047857');
}

function drawReceptionInterior(
  ctx: CanvasRenderingContext2D,
  room: OfficeRoom,
  x: number, y: number,
  scale: number,
  colors: RoomColors,
): void {
  const s = scale;
  const ts = TILE_SIZE * s;
  const innerX = x + ts;
  const innerY = y + ts;
  const innerTilesW = room.width - 2;
  const innerTilesH = room.height - 2;
  const innerW = innerTilesW * ts;
  const innerH = innerTilesH * ts;

  // ── Step 3: Runner rug in walkway ──
  const rugW = Math.min(innerTilesW - 2, 5);
  drawRug(ctx, innerX + (innerW - rugW * ts) / 2, innerY + innerH - ts * 1.5, s, rugW, 1);

  // ── Step 1: Anchor — Reception desk centered ──
  const deskW = Math.min(s * 20, innerW - s * 4);
  drawReceptionDesk(ctx, innerX + (innerW - deskW) / 2, innerY + ts * 1.2, s, colors);

  // ── Step 4: On desk — monitor + keyboard ──
  const deskCenterX = innerX + (innerW - deskW) / 2;
  drawComputerMonitor(ctx, deskCenterX + s * 3, innerY + ts * 1.2 - ts * 0.75, s);
  drawKeyboard(ctx, deskCenterX + s * 3, innerY + ts * 1.2 + s * 1, s);
  // Mug on reception desk
  drawMug(ctx, deskCenterX + deskW - ts, innerY + ts * 1.2 - ts * 0.15, s);
  // Paper stack on desk
  drawPaperStack(ctx, deskCenterX + ts, innerY + ts * 1.2 - ts * 0.1, s);

  // ── Step 2: Chair behind desk ──
  drawOfficeChair(ctx, deskCenterX + s * 8, innerY + ts * 0.2, s);

  // ── Step 2: Guest chairs (waiting area) ──
  if (innerTilesH >= 4) {
    const waitY = innerY + innerH - ts * 1.2;
    drawGuestChair(ctx, innerX + ts, waitY, s);
    drawGuestChair(ctx, innerX + ts * 2.2, waitY, s);
    if (innerTilesW >= 6) {
      drawGuestChair(ctx, innerX + ts * 3.4, waitY, s);
    }
  }

  // ── Step 2: Sofa in waiting area ──
  if (innerTilesW >= 6 && innerTilesH >= 5) {
    drawLoungeSofa(ctx, innerX + innerW - ts * 2.5, innerY + innerH - ts * 2.5, s);
  }

  // ── Step 4: Clutter — flanking plants ──
  drawPlantPot(ctx, innerX + s * 1, innerY + innerH - ts, s);
  drawPlantPot(ctx, innerX + innerW - ts, innerY + innerH - ts, s);

  // ── Bookshelf on back wall ──
  if (innerTilesW >= 5) {
    drawBookshelf(ctx, innerX + innerW - ts * 1.2, innerY + s * 1, s);
  }

  // ── Filing cabinet near desk ──
  drawFilingCabinet(ctx, innerX + s * 1, innerY + s * 1, s);

  // ── Plant on bookshelf (top) ──
  if (innerTilesW >= 5) {
    drawPlantPot(ctx, innerX + innerW - ts * 1.2 + s * 1, innerY - ts * 0.3, s);
  }

  // ── Wall decorations: company sign + clock ──
  drawCompanySign(ctx, innerX + (innerW - s * 10) / 2, innerY + s * 0.5, s, colors.accent);
  drawWallClock(ctx, innerX + innerW - s * 8, innerY + s * 1, s);
}

function drawManagerInterior(
  ctx: CanvasRenderingContext2D,
  room: OfficeRoom,
  x: number, y: number,
  scale: number,
  colors: RoomColors,
): void {
  const s = scale;
  const ts = TILE_SIZE * s;
  const innerX = x + ts;
  const innerY = y + ts;
  const innerTilesW = room.width - 2;
  const innerTilesH = room.height - 2;
  const innerW = innerTilesW * ts;
  const innerH = innerTilesH * ts;

  // ── Step 3: Large rug under desk area ──
  const rugW = Math.min(innerTilesW - 1, 5);
  const rugH = Math.min(innerTilesH - 1, 3);
  drawRug(ctx, innerX + (innerW - rugW * ts) / 2, innerY + (innerH - rugH * ts) / 2, s, rugW, rugH);

  // ── Step 1: Anchor — Large manager desk centered against back wall ──
  const deskX = innerX + (innerW - ts * 3) / 2;
  const deskY = innerY + ts * 0.3;
  drawManagerDesk(ctx, deskX, deskY, s);

  // ── Step 4: On desk — monitor + keyboard + mug + paper ──
  drawComputerMonitor(ctx, deskX + ts * 0.8, deskY - ts * 0.75, s);
  drawKeyboard(ctx, deskX + ts * 0.8, deskY + s * 1, s);
  drawMug(ctx, deskX + ts * 2.2, deskY - ts * 0.1, s);
  drawPaperStack(ctx, deskX + ts * 0.1, deskY - ts * 0.1, s);

  // ── Step 2: Executive chair behind desk ──
  drawExecutiveChair(ctx, deskX + ts * 1, deskY + ts * 1.1, s);

  // ── Step 2: Guest chairs in front ──
  if (innerTilesH >= 4) {
    drawGuestChair(ctx, innerX + ts * 1, innerY + innerH - ts * 2, s);
    drawGuestChair(ctx, innerX + innerW - ts * 2, innerY + innerH - ts * 2, s);
  }

  // ── Step 2: Bookshelf on side wall ──
  drawBookshelf(ctx, innerX + s * 1, innerY + s * 1, s);

  // ── Step 2: Filing cabinet on other side ──
  drawFilingCabinet(ctx, innerX + innerW - ts, innerY + s * 2, s);
  if (innerTilesH > 4) {
    drawFilingCabinet(ctx, innerX + innerW - ts, innerY + ts + s * 4, s);
  }

  // ── Step 4: Clutter — plants ──
  drawPlantPot(ctx, innerX + s * 1, innerY + innerH - ts, s);
  drawPlantPot(ctx, innerX + innerW - ts, innerY + innerH - ts, s);
  drawWasteBin(ctx, innerX + ts * 2, innerY + innerH - ts, s);

  // ── Wall decorations: 2 framed diplomas + wall clock ──
  drawFramedDiploma(ctx, innerX + innerW - s * 12, innerY + s * 0.5, s);
  drawFramedDiploma(ctx, innerX + innerW - s * 6, innerY + s * 0.5, s);
  drawWallClock(ctx, deskX + ts * 1.2, innerY + s * 0.5, s);
}

function drawBathroomInterior(
  ctx: CanvasRenderingContext2D,
  room: OfficeRoom,
  x: number, y: number,
  scale: number,
  _colors: RoomColors,
): void {
  const s = scale;
  const ts = TILE_SIZE * s;
  const innerX = x + ts;
  const innerY = y + ts;
  const innerTilesW = room.width - 2;
  const innerTilesH = room.height - 2;
  const innerW = innerTilesW * ts;
  const innerH = innerTilesH * ts;

  // ── Tile floor pattern (checker) ──
  for (let ty = 0; ty < innerTilesH; ty++) {
    for (let tx = 0; tx < innerTilesW; tx++) {
      const tileColor = (tx + ty) % 2 === 0 ? '#e2e8f0' : '#cbd5e1';
      px(ctx, innerX + tx * ts, innerY + ty * ts, ts, ts, tileColor);
    }
  }

  // ── Step 1: 2 stall partitions (left side) ──
  const stallH = ts * (innerTilesH - 1);
  drawStallPartition(ctx, innerX + ts * 1, innerY + s * 1, s, stallH * 0.7);
  if (innerTilesW >= 4) {
    drawStallPartition(ctx, innerX + ts * 2.2, innerY + s * 1, s, stallH * 0.7);
  }

  // ── Step 2: 2 sinks along back wall (right side) ──
  const sinkStartX = innerX + innerW - ts * 2;
  drawBathroomSink(ctx, sinkStartX, innerY + s * 2, s);
  drawBathroomSink(ctx, sinkStartX + ts, innerY + s * 2, s);

  // ── Mirror above sinks ──
  drawBathroomMirror(ctx, sinkStartX, innerY + s * 0.5, s);
  if (innerTilesW >= 4) {
    drawBathroomMirror(ctx, sinkStartX + ts * 0.8, innerY + s * 0.5, s);
  }

  // ── Hand dryer on wall ──
  drawHandDryer(ctx, innerX + innerW - s * 4, innerY + ts + s * 2, s);

  // ── Waste bin ──
  drawWasteBin(ctx, innerX + innerW - ts, innerY + innerH - ts, s);
}

// ─── Room cache — draw each room once to an OffscreenCanvas ─────────────────

const roomCanvasCache = new Map<string, OffscreenCanvas | HTMLCanvasElement>();

function getRoomCanvas(
  room: OfficeRoom,
  colors: RoomColors,
  scale: number,
): OffscreenCanvas | HTMLCanvasElement {
  const key = `${room.id}:${scale}`;
  const cached = roomCanvasCache.get(key);
  if (cached) return cached;

  const pixelW = room.width * TILE_SIZE * scale;
  const pixelH = room.height * TILE_SIZE * scale;

  let canvas: OffscreenCanvas | HTMLCanvasElement;
  if (typeof OffscreenCanvas !== 'undefined') {
    canvas = new OffscreenCanvas(pixelW, pixelH);
  } else {
    canvas = document.createElement('canvas');
    canvas.width = pixelW;
    canvas.height = pixelH;
  }

  const ctx = canvas.getContext('2d') as CanvasRenderingContext2D;
  if (!ctx) return canvas;
  (ctx as any).imageSmoothingEnabled = false;

  // Draw interior furniture based on room type
  switch (room.roomType) {
    case 'conference':
      drawConferenceInterior(ctx, room, 0, 0, scale, colors);
      break;
    case 'workspace':
      drawWorkspaceInterior(ctx, room, 0, 0, scale, colors);
      break;
    case 'lounge':
      drawLoungeInterior(ctx, room, 0, 0, scale, colors);
      break;
    case 'server':
      drawServerInterior(ctx, room, 0, 0, scale, colors);
      break;
    case 'kitchen':
      drawKitchenInterior(ctx, room, 0, 0, scale, colors);
      break;
    case 'reception':
      drawReceptionInterior(ctx, room, 0, 0, scale, colors);
      break;
    case 'manager':
      drawManagerInterior(ctx, room, 0, 0, scale, colors);
      break;
    case 'bathroom':
      drawBathroomInterior(ctx, room, 0, 0, scale, colors);
      break;
  }

  roomCanvasCache.set(key, canvas);
  return canvas;
}

/** Clear the room canvas cache (call when layout changes). */

export interface RoomRenderData {
  bgCanvas: OffscreenCanvas | HTMLCanvasElement;
  entities: RoomEntity[];
}

const roomRenderCache = new Map<string, RoomRenderData>();

export function getRoomRenderData(room: OfficeRoom, colors: RoomColors, scale: number): RoomRenderData {
  const key = `${room.id}:${scale}`;
  const cached = roomRenderCache.get(key);
  if (cached) return cached;

  const builder = new EntityBuilder();
  setActiveEntityBuilder(builder);
  
  const bgCanvas = getRoomCanvas(room, colors, scale);
  
  setActiveEntityBuilder(null);
  
  const data = { bgCanvas, entities: builder.entities };
  roomRenderCache.set(key, data);
  return data;
}

export function clearRoomCache(): void {
  roomCanvasCache.clear();
}

// ─── Pixel font fallback chain (for room labels) ──────────────────────────
const PIXEL_FONT = '"Press Start 2P", "Silkscreen", "VT323", "Courier New", "Courier", monospace';

// ─── Label color constants (matching pixel-renderer.ts COLORS) ────────────
const LABEL_COLORS = {
  label_bg: 'rgba(46, 52, 64, 0.75)',
  text_shadow: '#2E3440',
  text_light: '#E5E9F0',
};

// ─── Main draw function ─────────────────────────────────────────────────────

/**
 * Draw an office room onto a Canvas 2D context.
 * Blits the pre-cached room interior canvas, then draws labels.
 * Active state is handled by an outline in the main renderer.
 */
export function drawHouse(
  ctx: CanvasRenderingContext2D,
  house: GeneratedHouse,
  x: number,
  y: number,
  scale: number,
  label?: string,
  _isActive: boolean = false,
  room?: OfficeRoom,
): void {
  const { template, colors } = house;

  ctx.save();

  // ── Draw cached room interior (furniture) on top of tile floor ──
  if (room) {
    const renderData = getRoomRenderData(room, colors, scale);
    ctx.drawImage(renderData.bgCanvas as any, x, y);
  }

  // ── Label above the room (pixel-perfect text) ──
  if (label) {
    const totalW = template.width * scale;
    const fontSize = Math.max(8, Math.round(scale * 4));
    ctx.font = `${fontSize}px ${PIXEL_FONT}`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'bottom';

    const labelX = x + totalW / 2;
    const labelY = y + template.labelY * scale - scale * 2;

    const measured = ctx.measureText(label);
    const pillW = measured.width + scale * 4;
    const pillH = fontSize + scale * 2;
    const pillX = Math.round(labelX - pillW / 2);
    const pillY = Math.round(labelY - pillH);

    // Background — plain fillRect, no rounded rects for pixel art
    ctx.fillStyle = LABEL_COLORS.label_bg;
    ctx.fillRect(pillX, pillY, pillW, pillH);

    // Text shadow
    ctx.fillStyle = LABEL_COLORS.text_shadow;
    ctx.fillText(label, Math.round(labelX + scale), Math.round(labelY + scale));

    // Main text
    ctx.fillStyle = LABEL_COLORS.text_light;
    ctx.fillText(label, Math.round(labelX), Math.round(labelY));
  }

  ctx.restore();
}
