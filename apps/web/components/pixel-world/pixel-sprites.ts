/**
 * Pixel sprite fallback types and canvas helpers used while Phaser textures
 * initialize.
 */

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/** Simple deterministic string hash (djb2). */
export function hashStr(s: string): number {
  let h = 5381;
  for (let i = 0; i < s.length; i++) {
    h = ((h << 5) + h + s.charCodeAt(i)) | 0;
  }
  return h;
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type SpriteFrame = OffscreenCanvas | HTMLCanvasElement;

export interface AnimationSet {
  idle_down: SpriteFrame[];
  idle_up: SpriteFrame[];
  idle_left: SpriteFrame[];
  idle_right: SpriteFrame[];
  walk_down: SpriteFrame[];
  walk_up: SpriteFrame[];
  walk_left: SpriteFrame[];
  walk_right: SpriteFrame[];
  work_down: SpriteFrame[];
  work_up: SpriteFrame[];
  work_left: SpriteFrame[];
  work_right: SpriteFrame[];
  think_down: SpriteFrame[];
  think_up: SpriteFrame[];
  think_left: SpriteFrame[];
  think_right: SpriteFrame[];
}

export interface PixelSprite {
  frames: AnimationSet;
  palette: string[]; // Legacy compatibility 
  characterPalette: any; 
  archetype: string;
  description: string;
  spriteSheetName?: string;
}

const SPRITE_SIZE = 48; // Each frame is exactly 48x48

export function generateSprite(agentId: string): PixelSprite {
  // Phaser replaces this placeholder with the preloaded atlas or roster sheet.
  return getFallbackBlankSprite(agentId);
}

function getFallbackBlankSprite(agentId: string): PixelSprite {
    const f = document.createElement('canvas');
    f.width = SPRITE_SIZE;
    f.height = SPRITE_SIZE;
    const ctx = f.getContext('2d')!;
    ctx.fillStyle = '#FF00FF'; // Magenta Missingno block
    ctx.fillRect(8, 8, 32, 32);

    const arr = [f, f, f, f];

    return {
        frames: {
            idle_down: arr, idle_up: arr, idle_left: arr, idle_right: arr,
            walk_down: arr, walk_up: arr, walk_left: arr, walk_right: arr,
            work_down: arr, work_up: arr, work_left: arr, work_right: arr,
            think_down: arr, think_up: arr, think_left: arr, think_right: arr,
        },
        palette: ['#FF00FF'],
        characterPalette: {},
        archetype: 'Placeholder',
        description: `Placeholder for ${agentId}`
    };
}

// ---------------------------------------------------------------------------
// DrawOptions and drawSprite — consumed by pixel-renderer.ts
// ---------------------------------------------------------------------------

export interface DrawOptions {
  x: number;
  y: number;
  scale?: number;
  flipX?: boolean;
}

export function drawSprite(
  ctx: CanvasRenderingContext2D,
  frame: SpriteFrame,
  _palette: string[],
  opts: DrawOptions,
): void {
  const { x, y, scale = 1, flipX = false } = opts;
  const dx = Math.round(x);
  const dy = Math.round(y);
  const dw = Math.round(SPRITE_SIZE * scale);
  const dh = Math.round(SPRITE_SIZE * scale);

  ctx.save();
  ctx.imageSmoothingEnabled = false;

  if (flipX) {
    ctx.translate(dx + dw, dy);
    ctx.scale(-1, 1);
    ctx.drawImage(frame as any, 0, 0, dw, dh);
  } else {
    ctx.drawImage(frame as any, dx, dy, dw, dh);
  }

  ctx.restore();
}

// Provide a stub for legacy components expecting `drawSpriteFromSheet`
export function drawSpriteFromSheet(): boolean {
   return false;
}
