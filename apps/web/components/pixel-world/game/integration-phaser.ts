// integration-phaser.ts — Phaser-dependent bridge utilities.
//
// This module imports Phaser and MUST ONLY be imported from client-side code
// (i.e., inside Phaser scenes or dynamic imports). Never import this from a
// module that gets SSR-compiled.

import * as Phaser from 'phaser';
import { getTileAtlas, TILE_SIZE } from '../pixel-tiles';
import { getAgentVisualDescriptor, isChoruzRosterAsset } from '../agent-catalog';
import type { PixelSprite, SpriteFrame, AnimationSet } from '../pixel-sprites';
import { trace } from '../../../lib/api/choruz-trace';
import { usePixelWorldStore } from '../pixel-world-store';

// ---------------------------------------------------------------------------
// Tile atlas
// ---------------------------------------------------------------------------

/**
 * Load our procedurally-generated tile atlas into Phaser's texture manager.
 * Returns the texture key used for tileset creation.
 */
export function loadTileAtlasTexture(scene: Phaser.Scene): string {
  const TEXTURE_KEY = 'office-tiles';

  if (scene.textures.exists(TEXTURE_KEY)) {
    scene.textures.remove(TEXTURE_KEY);
  }

  const atlas = getTileAtlas();

  // getTileAtlas returns OffscreenCanvas | HTMLCanvasElement.
  // Phaser's addCanvas only accepts HTMLCanvasElement, so if we got an
  // OffscreenCanvas we need to blit it to a regular canvas.
  let htmlCanvas: HTMLCanvasElement;
  if (atlas instanceof HTMLCanvasElement) {
    htmlCanvas = atlas;
  } else {
    htmlCanvas = document.createElement('canvas');
    htmlCanvas.width = atlas.width;
    htmlCanvas.height = atlas.height;
    const ctx = htmlCanvas.getContext('2d');
    if (ctx) {
      ctx.drawImage(atlas as OffscreenCanvas, 0, 0);
    }
  }

  scene.textures.addCanvas(TEXTURE_KEY, htmlCanvas);
  return TEXTURE_KEY;
}

// ---------------------------------------------------------------------------
// Character sprite helpers
// ---------------------------------------------------------------------------

/** Width/height of each character sprite frame. */
export const SPRITE_PX = 48;

// Animation directions: each row in the 192×192 sprite sheet is one direction.
// 4 rows × 4 cols = 16 frames of 48×48 each.
// Row order for the generated atlas: down, right, up, left.
const ANIM_ROWS: { name: string; row: number }[] = [
  { name: 'down', row: 0 },
  { name: 'right', row: 1 },
  { name: 'up', row: 2 },
  { name: 'left', row: 3 },
];
const FRAMES_PER_ROW = 4;

// Roster sheet layout:
//   Row 0..3  walk cycles (down/right/up/left × 4 frames each)
//   Row 4     typing    (front-facing, 4 frames)
//   Row 5     thinking  (front-facing, 4 frames)
//   Row 6     sitting   (front-facing, 4 frames)
//   Row 7     talking   (front-facing, 4 frames)
const ROSTER_ROWS_TOTAL = 8;
const ROSTER_ROW_WALK = 4;   // rows 0-3 are walk × 4 directions
const ROSTER_ROW_TYPING = 4;
const ROSTER_ROW_THINKING = 5;
const ROSTER_ROW_SITTING = 6;
const ROSTER_ROW_TALKING = 7;

/** Width (px) and height (px) of a real preloaded Choruz roster sheet. */
const ROSTER_SHEET_W = SPRITE_PX * FRAMES_PER_ROW;         // 48 × 4 = 192
const ROSTER_SHEET_H = SPRITE_PX * ROSTER_ROWS_TOTAL;      // 48 × 8 = 384

/**
 * Authoritative check for "texture is backed by a real preloaded Choruz roster
 * sheet". Distinguishes from a procedural fallback canvas (flat strip of
 * `N×SPRITE_PX × SPRITE_PX`) that may live under the same `agent-<id>` key
 * after a previous missing-sheet recovery — we verify the underlying image
 * is actually 192×384 rather than trusting the key name or a dirty cache.
 */
function isRosterSheetTexture(tex: Phaser.Textures.Texture): boolean {
  const src = tex.source?.[0];
  if (!src) return false;
  return src.width === ROSTER_SHEET_W && src.height === ROSTER_SHEET_H;
}

/**
 * Load an agent's texture from the pre-loaded atlas.
 *
 * The atlas ('agent-atlas') is loaded in MainScene.preload().
 * Each agent's master asset maps to a 192×192 region in the atlas,
 * which we slice into 16 individual 48×48 frame sub-textures.
 *
 * Falls back to the old procedural canvas method if the atlas is unavailable.
 */
export function loadAgentTexture(
  scene: Phaser.Scene,
  agentId: string,
  sprite: PixelSprite,
): string {
  const key = `agent-${agentId}`;
  const descriptor = getAgentVisualDescriptor(agentId);
  const masterAsset = descriptor?.masterAsset;

  // Choruz roster path (checked BEFORE the generic remove-if-no-frame-0 logic
  // below, because MainScene.preload() loads each roster sheet under exactly
  // the `agent-<id>` key, and its frames haven't been sliced yet on the
  // first call — we must not blow it away).
  if (isChoruzRosterAsset(masterAsset) && scene.textures.exists(key)) {
    const tex = scene.textures.get(key);
    // Authoritative dimension check: only take the roster fast path if the
    // underlying texture image really is 192×384 (the 8-row roster sheet).
    // If a previous call's procedural fallback wrote a flat strip under
    // this key, skip this branch and let the non-roster cache check below
    // handle it.
    if (isRosterSheetTexture(tex)) {
      if (!tex.has('0')) {
        // 8 rows × 4 cols = 32 subframes (walk × 4 dirs + typing + thinking + sitting + talking).
        for (let row = 0; row < ROSTER_ROWS_TOTAL; row++) {
          for (let col = 0; col < FRAMES_PER_ROW; col++) {
            const idx = row * FRAMES_PER_ROW + col;
            tex.add(idx, 0, col * SPRITE_PX, row * SPRITE_PX, SPRITE_PX, SPRITE_PX);
          }
        }
      }
      return key;
    }
  }

  // Non-roster cache check: if a texture with this agent's key already has
  // the first sliced frame, reuse it. Otherwise clear the stale entry so the
  // legacy atlas / procedural path can rebuild it from scratch.
  if (scene.textures.exists(key)) {
    const tex = scene.textures.get(key);
    if (tex.has('0')) return key;
    scene.textures.remove(key);
  }

  // If the descriptor points at a roster sheet but its texture is missing
  // (preload race / 404), skip the atlas fallback — the atlas doesn't know
  // about this agent and would incorrectly masquerade it as "coder".
  // Go straight to the procedural canvas fallback below.
  const skipAtlasFallback = isChoruzRosterAsset(masterAsset);
  if (skipAtlasFallback) {
    const instanceId = usePixelWorldStore.getState().instanceId;
    trace.event('pixel_asset_fallback', {
      pixel_world_instance_id: instanceId,
      agent_id: agentId,
      asset_key: masterAsset,
      fallback_tier: 'procedural_canvas',
      reason: 'roster_sheet_missing',
    });
    console.warn(`[loadAgentTexture] roster sheet missing for '${agentId}', falling back to procedural canvas`);
  }

  const atlasFrameName = !skipAtlasFallback ? (masterAsset ?? 'coder') : undefined;

  if (atlasFrameName && scene.textures.exists('agent-atlas')) {
    const atlasTex = scene.textures.get('agent-atlas');
    if (atlasTex.has(atlasFrameName)) {
      // Get the atlas frame region for this agent's master asset
      const baseFrame = atlasTex.get(atlasFrameName);
      const bx = baseFrame.cutX;
      const by = baseFrame.cutY;

      // Create a new texture with 16 sub-frames (4 rows × 4 cols of 48×48)
      // We add frames to the atlas texture itself, namespaced by agent key
      const agentTex = scene.textures.get('agent-atlas');
      let frameIdx = 0;
      for (let row = 0; row < 4; row++) {
        for (let col = 0; col < FRAMES_PER_ROW; col++) {
          const fx = bx + col * SPRITE_PX;
          const fy = by + row * SPRITE_PX;
          // Register frame with a namespaced key: "agentId_frameIdx"
          if (!agentTex.has(`${agentId}_${frameIdx}`)) {
            agentTex.add(`${agentId}_${frameIdx}`, 0, fx, fy, SPRITE_PX, SPRITE_PX);
          }
          frameIdx++;
        }
      }
      return 'agent-atlas'; // Return atlas key — animations use namespaced frames
    }
  }

  // Fallback: old procedural canvas method
  const strips = sprite.frames;
  const allFrames: SpriteFrame[] = [];
  const orderedKeys: (keyof AnimationSet)[] = [
    'idle_down', 'idle_up', 'idle_left', 'idle_right',
    'walk_down', 'walk_up', 'walk_left', 'walk_right',
    'work_down', 'work_up', 'work_left', 'work_right',
    'think_down', 'think_up', 'think_left', 'think_right',
  ];
  for (const k of orderedKeys) {
    const strip = strips[k];
    if (strip) allFrames.push(...strip);
  }
  if (allFrames.length === 0) return key;

  const totalWidth = allFrames.length * SPRITE_PX;
  const canvas = document.createElement('canvas');
  canvas.width = totalWidth;
  canvas.height = SPRITE_PX;
  const ctx = canvas.getContext('2d');
  if (!ctx) return key;
  ctx.imageSmoothingEnabled = false;
  for (let i = 0; i < allFrames.length; i++) {
    ctx.drawImage(allFrames[i] as CanvasImageSource, i * SPRITE_PX, 0);
  }
  const texture = scene.textures.addCanvas(key, canvas);
  if (texture) {
    for (let i = 0; i < allFrames.length; i++) {
      texture.add(i, 0, i * SPRITE_PX, 0, SPRITE_PX, SPRITE_PX);
    }
  }
  return key;
}

/**
 * Create Phaser animations for an agent.
 *
 * When using the atlas, frames are `{agentId}_0` through `{agentId}_15`
 * on the 'agent-atlas' texture. Layout: 4 rows (down/right/up/left) × 4 cols.
 *
 * When using fallback canvas, frames are sequential integers on `agent-{agentId}`.
 */
export function createAgentAnimations(
  scene: Phaser.Scene,
  agentId: string,
  sprite: PixelSprite,
): void {
  const descriptor = getAgentVisualDescriptor(agentId);
  const masterAsset = descriptor?.masterAsset;

  // Choruz roster: frames are integers (0..31) on the per-agent texture, laid
  // out row-major over an 8-row sheet:
  //   rows 0-3  walk cycles (down/right/up/left × 4 frames each) → frames 0..15
  //   row  4    typing   (front-only, 4 frames) → frames 16..19
  //   row  5    thinking (front-only, 4 frames) → frames 20..23
  //   row  6    sitting  (front-only, 4 frames) → frames 24..27
  //   row  7    talking  (front-only, 4 frames) → frames 28..31
  //
  // The frame-index math is ONLY valid when the texture image is genuinely
  // 192×384 (the preloaded PNG). If the preload failed earlier and
  // loadAgentTexture wrote a procedural flat-strip canvas under the same
  // `agent-<id>` key, the dimension check fails and we fall through to the
  // legacy procedural-animation branch below (which understands the strip).
  if (isChoruzRosterAsset(masterAsset)) {
    const textureKey = `agent-${agentId}`;
    if (
      scene.textures.exists(textureKey) &&
      isRosterSheetTexture(scene.textures.get(textureKey))
    ) {
      // Rows 0-3: per-direction walk cycles. Each direction gets its own
      // idle / walk / work / think anim built from those 4 frames (legacy
      // compat — work / think play the same 4 walk frames at different rates).
      const DIRECTION_FRAMES: Record<string, number[]> = {
        down:  [0, 1, 2, 3],
        right: [4, 5, 6, 7],
        up:    [8, 9, 10, 11],
        left:  [12, 13, 14, 15],
      };
      const DIR_ANIM_TYPES = ['idle', 'walk', 'work', 'think'];
      const FRAME_RATES: Record<string, number> = {
        idle: 2, walk: 8, work: 4, think: 2,
        typing: 6, thinking: 3, sitting: 2, talking: 5,
      };

      for (const dir of Object.keys(DIRECTION_FRAMES)) {
        for (const animType of DIR_ANIM_TYPES) {
          const globalAnimKey = `${agentId}_${animType}_${dir}`;
          if (scene.anims.exists(globalAnimKey)) continue;

          const rowFrames = DIRECTION_FRAMES[dir];
          const sequence = rowFrames;

          scene.anims.create({
            key: globalAnimKey,
            frames: sequence.map((idx) => ({ key: textureKey, frame: idx })),
            frameRate: FRAME_RATES[animType] ?? 2,
            repeat: -1,
          });
        }
      }

      // Rows 4-7: status animations (typing / thinking / sitting / talking).
      // These are front-only (no directional variants). Registered under
      // `<agentId>_<action>` with no direction suffix, so callers that want
      // a status playback call the key directly; the Phaser sprite is assumed
      // to be facing down/south while playing (which matches the way the
      // Nano Banana strips were generated).
      const framesForRow = (row: number): number[] =>
        [0, 1, 2, 3].map((c) => row * FRAMES_PER_ROW + c);
      const STATUS_ROW_FRAMES: Record<string, number[]> = {
        typing:   framesForRow(ROSTER_ROW_TYPING),
        thinking: framesForRow(ROSTER_ROW_THINKING),
        sitting:  framesForRow(ROSTER_ROW_SITTING),
        talking:  framesForRow(ROSTER_ROW_TALKING),
      };

      for (const [action, frames] of Object.entries(STATUS_ROW_FRAMES)) {
        const statusAnimKey = `${agentId}_${action}`;
        if (scene.anims.exists(statusAnimKey)) continue;
        scene.anims.create({
          key: statusAnimKey,
          frames: frames.map((idx) => ({ key: textureKey, frame: idx })),
          frameRate: FRAME_RATES[action] ?? 4,
          repeat: -1,
        });
      }
      return;
    }
    // Roster texture missing OR it was built by the procedural fallback
    // (flat strip layout) — fall through to the procedural-animation branch
    // below so the caller still gets basic strip-based animations driven
    // by sprite.frames.
  }

  // If this is a roster agent whose sheet didn't load, DO NOT let the code
  // below masquerade it as "coder" via the atlas-fallback default. Skip the
  // atlas path entirely and go straight to the procedural strip branch.
  const rosterFallback = isChoruzRosterAsset(masterAsset);

  const atlasFrameName = !rosterFallback ? (masterAsset ?? 'coder') : undefined;
  const useAtlas = atlasFrameName && scene.textures.exists('agent-atlas') &&
    scene.textures.get('agent-atlas').has(atlasFrameName);

  if (useAtlas) {
    // Atlas mode: 4 rows × 4 frames, texture key is 'agent-atlas',
    // frame names are '{agentId}_0' .. '{agentId}_15'
    const textureKey = 'agent-atlas';
    const DIRECTION_FRAMES: Record<string, number[]> = {
      down:  [0, 1, 2, 3],
      right: [4, 5, 6, 7],
      up:    [8, 9, 10, 11],
      left:  [12, 13, 14, 15],
    };
    const ANIM_TYPES = ['idle', 'walk', 'work', 'think'];
    const FRAME_RATES: Record<string, number> = { idle: 2, walk: 8, work: 4, think: 2 };

    for (const dir of Object.keys(DIRECTION_FRAMES)) {
      for (const animType of ANIM_TYPES) {
        const globalAnimKey = `${agentId}_${animType}_${dir}`;
        if (scene.anims.exists(globalAnimKey)) continue;

        const rowFrames = DIRECTION_FRAMES[dir];
        const sequence = animType === 'walk'
          ? [rowFrames[0], rowFrames[1], rowFrames[2], rowFrames[1]]
          : rowFrames;

        const frames = sequence.map((idx) => ({
          key: textureKey,
          frame: `${agentId}_${idx}`,
        }));

        scene.anims.create({
          key: globalAnimKey,
          frames,
          frameRate: FRAME_RATES[animType] ?? 2,
          repeat: -1,
        });
      }
    }
  } else {
    // Fallback: old sequential-frame canvas mode
    const orderedKeys: (keyof AnimationSet)[] = [
      'idle_down', 'idle_up', 'idle_left', 'idle_right',
      'walk_down', 'walk_up', 'walk_left', 'walk_right',
      'work_down', 'work_up', 'work_left', 'work_right',
      'think_down', 'think_up', 'think_left', 'think_right',
    ];
    const textureKey = `agent-${agentId}`;
    let frameOffset = 0;
    for (const animKey of orderedKeys) {
      const strip = sprite.frames[animKey];
      if (!strip || strip.length === 0) continue;
      const globalAnimKey = `${agentId}_${animKey}`;
      if (scene.anims.exists(globalAnimKey)) { frameOffset += strip.length; continue; }
      const frames = [];
      for (let i = 0; i < strip.length; i++) {
        frames.push({ key: textureKey, frame: frameOffset + i });
      }
      let frameRate = 2;
      if (animKey.startsWith('walk_')) frameRate = 8;
      else if (animKey.startsWith('work_')) frameRate = 4;
      scene.anims.create({ key: globalAnimKey, frames, frameRate, repeat: -1 });
      frameOffset += strip.length;
    }
  }
}
