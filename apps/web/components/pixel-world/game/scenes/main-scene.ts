// main-scene.ts — Core Phaser scene for the pixel-world office visualization.
//
// Responsibilities:
//  1. Render the world via a pre-baked floor image (plus an optional
//     procedural TileGrid fallback when no floor asset is available).
//  2. Spawn a controllable player sprite (WASD movement).
//  3. Camera follow with pixel-perfect snapping.
//  4. Y-sort all character sprites by depth.
//  5. Create NPC agent sprites, drawing from one of three texture sources:
//       - shared `agent-atlas` (17 legacy 192×192 regions)
//       - per-agent Choruz roster sheets (192×384)
//       - procedural canvas fallback
//     and register directional walk/idle/work/think plus front-only
//     typing/thinking/sitting/talking animations as available.
//  6. Click/hover events emitted to EventBus for React consumption.

import * as Phaser from 'phaser';
import {
  EventBus,
  EVT_AGENT_CLICKED,
  EVT_ROOM_CLICKED,
  EVT_ROOM_ENTERED,
  EVT_SCENE_READY,
  EVT_FOCUS_AGENT,
  EVT_HIGHLIGHT_ROOM,
  EVT_WORLD_DATA_CHANGED,
} from '../event-bus';
import {
  getStoreState,
  subscribeStore,
} from '../integration';
import {
  loadAgentTexture,
  createAgentAnimations,
  SPRITE_PX,
} from '../integration-phaser';
import { TILE_SIZE } from '../../pixel-tiles';
import { drawHouse, getRoomRenderData } from '../../pixel-houses';
import { CHORUZ_AGENT_SHEETS, getAgentVisualDescriptor } from '../../agent-catalog';
import { isStatusAnimState } from '../../pixel-animations';
import type { HouseInfo, PixelAgentState } from '../../pixel-world-store';
import { usePixelWorldStore } from '../../pixel-world-store';
import { trace } from '../../../../lib/api/choruz-trace';

function pixEventMs(name: string, data?: Record<string, unknown>): void {
  const instanceId = usePixelWorldStore.getState().instanceId;
  trace.event(name, { pixel_world_instance_id: instanceId, ...(data ?? {}) });
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const PLAYER_SPEED = 200; // pixels per second
const CAMERA_LERP = 0.08;
const CAMERA_ZOOM = 1.0;
const NPC_WANDER_INTERVAL = 3000; // ms between NPC wander decisions
const ROOM_HIGHLIGHT_ALPHA = 0.15;
const FLOOR_TRANSITION_COOLDOWN_MS = 600;
const FLOOR_FADE_DURATION_MS = 260;
const FLOOR_TRANSITION_MOVE_LOCK_MS = 1000;

type RectZone = {
  x: number;
  y: number;
  width: number;
  height: number;
};

type FloorConfig = {
  id: 'floor1' | 'floor2';
  bgKey: string;
  maskKey: string;
  bgSrc: string;
  maskSrc: string;
  triggerZone: RectZone;
  targetFloor: 'floor1' | 'floor2';
  targetSpawn: { x: number; y: number };
};

const FLOOR_CONFIGS: Record<FloorConfig['id'], FloorConfig> = {
  floor1: {
    id: 'floor1',
    bgKey: 'floor1-bg',
    maskKey: 'floor1-mask',
    bgSrc: '/game/floors/floor1/floor1_bg.png',
    maskSrc: '/game/floors/floor1/floor1_mask.png',
    triggerZone: { x: 1640, y: 1340, width: 170, height: 52 },
    targetFloor: 'floor2',
    targetSpawn: { x: 1720, y: 1432 },
  },
  floor2: {
    id: 'floor2',
    bgKey: 'floor2-bg',
    maskKey: 'floor2-mask',
    bgSrc: '/game/floors/floor2/floor2_bg.png',
    maskSrc: '/game/floors/floor2/floor2_mask.png',
    triggerZone: { x: 1645, y: 1352, width: 150, height: 45 },
    targetFloor: 'floor1',
    targetSpawn: { x: 1725, y: 1448 },
  },
};

// ---------------------------------------------------------------------------
// Scene
// ---------------------------------------------------------------------------

export class MainScene extends Phaser.Scene {
  // -- Floor image (composite canvas, replaces Phaser tilemap) --
  private floorImg: Phaser.GameObjects.Image | null = null;
  private sceneWorldWidth = 0;
  private sceneWorldHeight = 0;
  private worldScaleX = 1;
  private worldScaleY = 1;
  private maskPixels: Uint8ClampedArray | null = null;
  private maskWidth = 0;
  private maskHeight = 0;
  private currentFloorId: FloorConfig['id'] = 'floor1';
  private floorTransitionAt = 0;
  private floorTransitionInFlight = false;
  private floorFadeOverlay: Phaser.GameObjects.Rectangle | null = null;
  private playerMovementLockedUntil = 0;

  // -- Player --
  private player!: Phaser.GameObjects.Sprite;
  private playerSpeed = PLAYER_SPEED;
  private wasd!: {
    W: Phaser.Input.Keyboard.Key;
    A: Phaser.Input.Keyboard.Key;
    S: Phaser.Input.Keyboard.Key;
    D: Phaser.Input.Keyboard.Key;
  };
  private windowKeyState = { W: false, A: false, S: false, D: false };
  private windowKeyDownHandler: ((event: KeyboardEvent) => void) | null = null;
  private windowKeyUpHandler: ((event: KeyboardEvent) => void) | null = null;
  private cursors!: Phaser.Types.Input.Keyboard.CursorKeys;

  // -- NPC agents --
  public agentSprites: Map<string, Phaser.GameObjects.Sprite> = new Map();
  private agentNameLabels: Map<string, Phaser.GameObjects.Text> = new Map();
  private agentBubbles: Map<string, Phaser.GameObjects.Text> = new Map();
  private agentFloorAssignments: Map<string, FloorConfig['id']> = new Map();
  private furnitureSprites: Phaser.GameObjects.Image[] = [];

  // -- Y-sort container --
  // Sprites added directly to scene (no container — avoids rendering quirks)

  // -- Room highlights --
  private roomHighlights: Map<string, Phaser.GameObjects.Rectangle> = new Map();
  private activeRoomId: string | null = null;

  // -- Store sync --
  private unsubscribe: (() => void) | null = null;
  private lastAgentCount = 0;
  private lastHouseCount = 0;

  // -- NPC wander timer --
  private wanderTimer: Phaser.Time.TimerEvent | null = null;

  // -- Juiciness Polish --
  private lightingLayer!: Phaser.GameObjects.Rectangle;
  private playerShadow!: Phaser.GameObjects.Ellipse;
  private agentShadows: Map<string, Phaser.GameObjects.Ellipse> = new Map();
  private dustEmitter!: Phaser.GameObjects.Particles.ParticleEmitter;

  // -- Camera drag --
  private isCameraDragging = false;
  private dragPointerId: number | null = null;
  private dragStartScreenX = 0;
  private dragStartScreenY = 0;
  private dragStartScrollX = 0;
  private dragStartScrollY = 0;
  private suppressSelectionUntil = 0;

  constructor() {
    super({ key: 'MainScene' });
  }

  // =========================================================================
  // Lifecycle
  // =========================================================================

  preload(): void {
    // Log any load failure so confusing runtime fallbacks (frozen sprites,
    // missing anim keys, etc.) surface as explicit errors in the console.
    this.load.on(Phaser.Loader.Events.FILE_LOAD_ERROR, (file: Phaser.Loader.File) => {
      pixEventMs('pixel_asset_load_error', {
        asset_key: file.key,
        asset_type: file.type,
        asset_src: file.src,
      });
      console.error(`[MainScene.preload] failed to load ${file.key} from ${file.src}`);
    });

    // Load the massive 192x192 per-agent atlas (legacy 17 agents)
    this.load.atlas('agent-atlas', '/sprites/agents/agent_atlas.png', '/sprites/agents/agent_atlas.json');

    // Load the 20 redesigned Choruz roster sheets (each 192×384, 8 rows:
    // walk × 4 dirs + typing + thinking + sitting + talking).
    for (const [id, path] of Object.entries(CHORUZ_AGENT_SHEETS)) {
      this.load.image(`agent-${id}`, path);
    }

    // The local "player" avatar shares one of the roster sheets (see
    // agent-catalog.ts). Preload it under the `agent-player` key so
    // loadAgentTexture('player', …) finds it instead of falling through to
    // the magenta procedural placeholder.
    const playerAsset = getAgentVisualDescriptor('player').masterAsset;
    this.load.image('agent-player', playerAsset);

    for (const floor of Object.values(FLOOR_CONFIGS)) {
      this.load.image(floor.bgKey, floor.bgSrc);
      this.load.image(floor.maskKey, floor.maskSrc);
    }
  }

  create(): void {
    this.cameras.main.setAlpha(0);

    // No container — sprites added directly to scene for reliable rendering

    // Build the world from the current store state
    this.buildWorld().catch((err) => {
      const msg = err instanceof Error ? err.message : String(err);
      pixEventMs('pixel_world_scene_error', {
        stage: 'build_world',
        error: msg,
        stack: err instanceof Error ? err.stack?.slice(0, 4000) : undefined,
      });
      console.error('[MainScene] buildWorld failed:', err);
    });

    // Set up input
    this.setupInput();

    // Set up camera
    this.setupCamera();
    this.setupCameraDrag();

    // Set up event listeners (React -> Phaser)
    this.setupEventListeners();

    // Set up store sync
    this.setupStoreSync();

    // Set up NPC wander behavior
    this.setupNPCWander();

    // Signal React that the scene is ready
    EventBus.emit(EVT_SCENE_READY, this);

    // Expose internals for automated testing
    if (typeof window !== 'undefined') {
      (window as any).__PHASER_READY = true;
      (window as any).__PHASER_SCENE = this;
    }
  }

  update(time: number, delta: number): void {
    // Advance the store's agent animation state machine ( walk interpolation,
    // state transitions, frame cycling).  Without this, agents never move.
    const state = getStoreState();
    state.tick();

    this.updateLighting();

    this.handlePlayerMovement(time, delta);
    this.ySortDepth();
    this.syncNPCPositions(time);
  }

  shutdown(): void {
    if (this.unsubscribe) {
      this.unsubscribe();
      this.unsubscribe = null;
    }
    this.teardownInput();
    EventBus.removeAllListeners();
  }

  // =========================================================================
  // World construction
  // =========================================================================

  private async buildWorld(): Promise<void> {
    const state = getStoreState();
    const { tileGrid, houses, agents } = state;

    if (!tileGrid) {
      console.warn('[MainScene] No tileGrid yet, skipping buildWorld');
      return;
    }

    const floor = this.getCurrentFloorConfig();

    if (this.textures.exists(floor.bgKey)) {
      const bgSource = this.getTextureSource(floor.bgKey);
      if (!bgSource) {
        throw new Error('Custom floor background failed to load');
      }

      const mapW = bgSource.width;
      const mapH = bgSource.height;
      this.sceneWorldWidth = mapW;
      this.sceneWorldHeight = mapH;
      this.worldScaleX = state.worldWidth > 0 ? mapW / state.worldWidth : 1;
      this.worldScaleY = state.worldHeight > 0 ? mapH / state.worldHeight : 1;
      this.captureMaskPixels(floor.maskKey);

      this.floorImg = this.add.image(0, 0, floor.bgKey);
      this.floorImg.setOrigin(0, 0);
      this.floorImg.setDepth(-10);

      this.physics.world.setBounds(0, 0, mapW, mapH);
      this.cameras.main.setBounds(0, 0, mapW, mapH);

      this.createJuiceEffects(mapW, mapH);
      this.spawnPlayer(state);
      this.spawnAgents(agents);
      this.revealCamera();

      this.lastAgentCount = agents.size;
      this.lastHouseCount = state.houses.size;
      return;
    }

    // Phaser Canvas renderer + addCanvas textures + data tilemaps don't work
    // together (tileset.image stays undefined). Render the entire floor to
    // a single canvas and add as a plain Image.
    const { renderFullMapCanvas } = await import('../../pixel-tiles');
    const floorCanvas = renderFullMapCanvas(tileGrid);
    const floorKey = 'floor-composite';
    if (this.textures.exists(floorKey)) this.textures.remove(floorKey);
    this.textures.addCanvas(floorKey, floorCanvas);
    this.floorImg = this.add.image(0, 0, floorKey);
    this.floorImg.setOrigin(0, 0);
    this.floorImg.setDepth(-10);

    // Set world bounds
    const mapW = tileGrid.cols * TILE_SIZE;
    const mapH = tileGrid.rows * TILE_SIZE;
    this.physics.world.setBounds(0, 0, mapW, mapH);
    this.cameras.main.setBounds(0, 0, mapW, mapH);

    // -- Create room highlight rectangles --
    this.createRoomHighlights(houses);

    // -- Spawn Furniture / Houses (Layered over the grid floor) --
    this.createHouseTextures(houses);

    // -- Visual Polish (Juiciness effects) --
    this.createJuiceEffects(mapW, mapH);

    // -- Spawn player --
    this.spawnPlayer(state);

    // -- Spawn NPC agents --
    this.spawnAgents(agents);
    this.revealCamera();

    this.lastAgentCount = agents.size;
    this.lastHouseCount = houses.size;
  }

  private revealCamera(): void {
    this.cameras.main.setAlpha(1);
  }

  private snapCameraToPlayer(player: Phaser.GameObjects.Sprite): void {
    const cam = this.cameras.main;
    cam.stopFollow();
    cam.centerOn(player.x, player.y);
    cam.startFollow(player, true, 1, 1);
    this.time.delayedCall(0, () => {
      if (!this.player || this.player !== player) return;
      cam.startFollow(player, true, CAMERA_LERP, CAMERA_LERP);
    });
  }

  private getTextureSource(key: string): { width: number; height: number } | null {
    if (!this.textures.exists(key)) return null;
    const texture = this.textures.get(key);
    const source = texture.getSourceImage() as { width?: number; height?: number } | undefined;
    if (!source?.width || !source?.height) return null;
    return { width: source.width, height: source.height };
  }

  private getCurrentFloorConfig(): FloorConfig {
    return FLOOR_CONFIGS[this.currentFloorId];
  }

  private captureMaskPixels(maskKey: string): void {
    const texture = this.textures.get(maskKey);
    const source = texture.getSourceImage() as CanvasImageSource & { width: number; height: number };
    const canvas = document.createElement('canvas');
    canvas.width = source.width;
    canvas.height = source.height;
    const ctx = canvas.getContext('2d');
    if (!ctx) {
      this.maskPixels = null;
      this.maskWidth = 0;
      this.maskHeight = 0;
      return;
    }
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(source, 0, 0);
    const imageData = ctx.getImageData(0, 0, canvas.width, canvas.height);
    this.maskPixels = imageData.data;
    this.maskWidth = canvas.width;
    this.maskHeight = canvas.height;

    // Share the mask with the store so agent A* can pathfind on the same
    // walkability data the player uses. Without this, agents fall back to
    // the BSP-generated TileGrid which doesn't match the visible floor.
    usePixelWorldStore.getState().setWalkabilityMask(
      imageData.data,
      canvas.width,
      canvas.height,
    );
  }

  private worldToSceneX(x: number): number {
    return x * this.worldScaleX;
  }

  private worldToSceneY(y: number): number {
    return y * this.worldScaleY;
  }

  private sceneToWorldX(x: number): number {
    return this.worldScaleX !== 0 ? x / this.worldScaleX : x;
  }

  private sceneToWorldY(y: number): number {
    return this.worldScaleY !== 0 ? y / this.worldScaleY : y;
  }

  private isWalkableAt(x: number, y: number): boolean {
    if (!this.maskPixels || this.maskWidth === 0 || this.maskHeight === 0) return true;

    const samplePoints: Array<[number, number]> = [
      [x, y + 6],
      [x - 8, y + 4],
      [x + 8, y + 4],
      [x, y - 2],
    ];

    for (const [sx, sy] of samplePoints) {
      const px = Math.round(sx);
      const py = Math.round(sy);
      if (px < 0 || px >= this.maskWidth || py < 0 || py >= this.maskHeight) {
        return false;
      }

      const idx = (py * this.maskWidth + px) * 4;
      const r = this.maskPixels[idx];
      const g = this.maskPixels[idx + 1];
      const b = this.maskPixels[idx + 2];
      const a = this.maskPixels[idx + 3];
      if (a < 16) return false;

      const isWalkable = r > 240 && g > 240 && b > 240;
      if (!isWalkable) return false;
    }

    return true;
  }

  private isInsideZone(x: number, y: number, zone: RectZone): boolean {
    return x >= zone.x && x <= zone.x + zone.width && y >= zone.y && y <= zone.y + zone.height;
  }

  private maybeTriggerFloorTransition(): void {
    if (!this.player || this.floorTransitionInFlight) return;
    if (this.time.now - this.floorTransitionAt < FLOOR_TRANSITION_COOLDOWN_MS) return;

    const floor = this.getCurrentFloorConfig();
    if (!this.isInsideZone(this.player.x, this.player.y, floor.triggerZone)) return;

    this.transitionToFloor(floor.targetFloor, floor.targetSpawn);
  }

  private transitionToFloor(targetFloorId: FloorConfig['id'], targetSpawn: { x: number; y: number }): void {
    if (this.floorTransitionInFlight) return;

    const targetFloor = FLOOR_CONFIGS[targetFloorId];
    const targetSource = this.getTextureSource(targetFloor.bgKey);
    if (!targetSource || !this.floorImg || !this.player) return;

    this.floorTransitionInFlight = true;
    this.floorTransitionAt = this.time.now;
    this.clearMovementInputState();

    const overlay = this.floorFadeOverlay ?? this.add.rectangle(
      0,
      0,
      this.scale.width,
      this.scale.height,
      0x000000,
      0,
    );
    overlay.setOrigin(0, 0);
    overlay.setScrollFactor(0);
    overlay.setDepth(200000);
    overlay.setAlpha(0);
    this.floorFadeOverlay = overlay;

    this.tweens.add({
      targets: overlay,
      alpha: 1,
      duration: FLOOR_FADE_DURATION_MS,
      ease: 'Quad.easeIn',
      onComplete: () => {
        this.currentFloorId = targetFloorId;
        this.sceneWorldWidth = targetSource.width;
        this.sceneWorldHeight = targetSource.height;
        this.captureMaskPixels(targetFloor.maskKey);
        this.floorImg?.setTexture(targetFloor.bgKey);
        this.physics.world.setBounds(0, 0, targetSource.width, targetSource.height);
        this.cameras.main.setBounds(0, 0, targetSource.width, targetSource.height);
        this.player.x = targetSpawn.x;
        this.player.y = targetSpawn.y;
        this.playerMovementLockedUntil = this.time.now + FLOOR_TRANSITION_MOVE_LOCK_MS;
        this.clearMovementInputState();

        const store = getStoreState();
        if (store.player) {
          store.player.x = this.sceneToWorldX(targetSpawn.x);
          store.player.y = this.sceneToWorldY(targetSpawn.y);
        }
        if (this.playerShadow) {
          this.playerShadow.x = targetSpawn.x;
          this.playerShadow.y = targetSpawn.y + 2;
        }
        this.cameras.main.centerOn(targetSpawn.x, targetSpawn.y);

        this.tweens.add({
          targets: overlay,
          alpha: 0,
          duration: FLOOR_FADE_DURATION_MS,
          ease: 'Quad.easeOut',
          onComplete: () => {
            this.floorTransitionInFlight = false;
          },
        });
      },
    });
  }

  /**
   * Tear down existing world objects and rebuild.
   * Called when the store data changes significantly.
   */
  private rebuildWorld(): void {
    this.cameras.main.setAlpha(0);

    // Clean up old objects
    // Destroy old sprites directly
    this.agentNameLabels.forEach(l => l.destroy());
    this.agentNameLabels.clear();
    this.agentBubbles.forEach(b => b.destroy());
    this.agentBubbles.clear();
    this.agentFloorAssignments.clear();
    this.furnitureSprites.forEach(s => s.destroy());
    this.furnitureSprites = [];
    this.agentSprites.forEach(s => s.destroy());
    if (this.player) this.player.destroy();
    this.agentSprites.clear();

    for (const rect of this.roomHighlights.values()) {
      rect.destroy();
    }
    this.roomHighlights.clear();

    if (this.playerShadow) this.playerShadow.destroy();
    this.agentShadows.forEach(s => s.destroy());
    this.agentShadows.clear();
    if (this.lightingLayer) this.lightingLayer.destroy();
    if (this.dustEmitter) this.dustEmitter.destroy();

    if (this.floorImg) {
      this.floorImg.destroy();
      this.floorImg = null;
    }

    // Rebuild
    this.buildWorld()
      .then(() => {
        // Re-attach camera follow to the new player sprite
        if (this.player) {
          this.snapCameraToPlayer(this.player);
        }
      })
      .catch((err) => {
        const msg = err instanceof Error ? err.message : String(err);
        pixEventMs('pixel_world_scene_error', {
          stage: 'rebuild_world',
          error: msg,
          stack: err instanceof Error ? err.stack?.slice(0, 4000) : undefined,
        });
        console.error('[MainScene] rebuildWorld failed:', err);
      });
  }

  // =========================================================================
  // Juiciness & Polish
  // =========================================================================

  private createJuiceEffects(mapW: number, mapH: number): void {
    // 1. Time-of-day global lighting layer
    this.lightingLayer = this.add.rectangle(0, 0, mapW, mapH, 0xffffff, 1);
    this.lightingLayer.setOrigin(0, 0);
    this.lightingLayer.setDepth(99999); // Top most layer
    this.lightingLayer.setBlendMode(Phaser.BlendModes.MULTIPLY);

    // 2. Footstep dust particles
    const ptGfx = this.add.graphics();
    ptGfx.fillStyle(0xddccaa, 0.8);
    ptGfx.fillCircle(2, 2, 2);
    ptGfx.generateTexture('dust-pt', 4, 4);
    ptGfx.destroy();

    this.dustEmitter = this.add.particles(0, 0, 'dust-pt', {
      scale: { start: 1, end: 0 },
      alpha: { start: 0.8, end: 0 },
      lifespan: 500,
      speed: { min: 5, max: 15 },
      angle: { min: 250, max: 290 },
      emitting: false,
    });
    this.dustEmitter.setDepth(-8); // Above floor & furniture (-9), below characters
  }

  private updateLighting(): void {
    if (!this.lightingLayer) return;
    this.lightingLayer.setFillStyle(0xffffff, 1);
  }

  // =========================================================================
  // Room highlights
  // =========================================================================

  private createRoomHighlights(houses: Map<string, HouseInfo>): void {
    for (const house of houses.values()) {
      const x = house.room.tileX * TILE_SIZE;
      const y = house.room.tileY * TILE_SIZE;
      const w = house.room.width * TILE_SIZE;
      const h = house.room.height * TILE_SIZE;

      const rect = this.add.rectangle(
        x + w / 2, y + h / 2, w, h,
        0x4488ff, 0,
      );
      rect.setDepth(-5);
      rect.setInteractive();
      rect.on('pointerup', () => {
        if (this.shouldSuppressSelection()) return;
        EventBus.emit(EVT_ROOM_CLICKED, house.id);
      });
      rect.on('pointerover', () => {
        rect.setAlpha(ROOM_HIGHLIGHT_ALPHA);
      });
      rect.on('pointerout', () => {
        if (this.activeRoomId !== house.id) {
          rect.setAlpha(0);
        }
      });

      this.roomHighlights.set(house.id, rect);
    }
  }

  private setActiveRoom(roomId: string | null): void {
    // Clear previous highlight
    if (this.activeRoomId) {
      const prev = this.roomHighlights.get(this.activeRoomId);
      if (prev) prev.setAlpha(0);
    }

    this.activeRoomId = roomId;

    if (roomId) {
      const rect = this.roomHighlights.get(roomId);
      if (rect) rect.setAlpha(ROOM_HIGHLIGHT_ALPHA);
    }
  }

  // =========================================================================
  // Furniture Houses
  // =========================================================================

  private createHouseTextures(houses: Map<string, HouseInfo>): void {
    // Clear old furniture
    this.furnitureSprites.forEach(s => s.destroy());
    this.furnitureSprites = [];

    for (const house of houses.values()) {
      const w = house.room.width * TILE_SIZE;
      const h = house.room.height * TILE_SIZE;

      const canvas = document.createElement('canvas');
      canvas.width = w;
      canvas.height = h;
      const ctx = canvas.getContext('2d');
      if (ctx) {
        drawHouse(ctx, house.house, 0, 0, 1, house.name, false, house.room);
      }

      const texKey = `house-${house.id}`;
      if (this.textures.exists(texKey)) {
        this.textures.remove(texKey);
      }
      this.textures.addCanvas(texKey, canvas);

      const img = this.add.image(house.worldX, house.worldY, texKey);
      img.setOrigin(0, 0);
      img.setDepth(-9); // Floor is -10, Characters are > 0

      // Extract furniture entities for actual Y-Sorting
      const renderData = getRoomRenderData(house.room, house.house.colors, 1);
      renderData.entities.forEach((entity, idx) => {
        const fTexKey = `furn-${house.id}-${idx}`;
        if (this.textures.exists(fTexKey)) this.textures.remove(fTexKey);
        this.textures.addCanvas(fTexKey, entity.canvas as any);
        
        const fx = house.worldX + entity.dx;
        const fy = house.worldY + entity.dy;
        
        const fSprite = this.add.image(fx, fy, fTexKey);
        fSprite.setOrigin(0, 0);
        // Depth is its Y footprint
        fSprite.setDepth(fy + entity.depthY);
        this.furnitureSprites.push(fSprite);
      });
    }
  }

  // =========================================================================
  // Player
  // =========================================================================

  private spawnPlayer(state: ReturnType<typeof getStoreState>): void {
    const playerState = state.player;
    let startX = state.worldWidth / 2;
    let startY = state.worldHeight / 2;

    if (playerState) {
      startX = playerState.x;
      startY = playerState.y;
    }

    startX = this.worldToSceneX(startX);
    startY = this.worldToSceneY(startY);

    // Load player texture from the store's player sprite
    const playerAgent = state.agents.get('player');
    let textureKey = '__DEFAULT';

    if (playerAgent) {
      try {
        textureKey = loadAgentTexture(this, 'player', playerAgent.sprite);
        createAgentAnimations(this, 'player', playerAgent.sprite);
        // Player texture loaded successfully
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        pixEventMs('pixel_asset_load_error', {
          asset_key: 'player_texture',
          stage: 'load_player_texture',
          error: msg,
        });
        console.error('[MainScene] Failed to load player texture:', err);
      }
    } else {
      pixEventMs('pixel_world_scene_error', {
        stage: 'no_player_in_store',
        error: 'no_player_agent',
      });
      console.warn('[MainScene] No player agent in store');
    }

    if (textureKey === '__DEFAULT' || !this.textures.exists(textureKey)) {
      // Fallback: generate a simple colored rectangle texture
      const gfx = this.add.graphics();
      gfx.fillStyle(0xffd700, 1);
      gfx.fillRect(0, 0, SPRITE_PX, SPRITE_PX);
      gfx.generateTexture('player-fallback', SPRITE_PX, SPRITE_PX);
      gfx.destroy();
      textureKey = 'player-fallback';
    }

    // When using atlas, the initial frame must be the namespaced string frame
    // (e.g. 'player_0'), not the numeric index 0 (which is the atlas __BASE).
    const initialFrame = textureKey === 'agent-atlas' ? 'player_0' : 0;
    this.player = this.add.sprite(startX, startY, textureKey, initialFrame);
    this.player.setScale(1.2);
    this.player.setOrigin(0.5, 0.75); // foot-aligned for y-sorting

    if (typeof window !== 'undefined') {
      (window as any).__PHASER_PLAYER = this.player;
    }

    this.playerShadow = this.add.ellipse(startX, startY, 14, 6, 0x000000, 0.25);
    this.playerShadow.setDepth(-8.5); // Under player, over floor/houses

    this.snapCameraToPlayer(this.player);
  }

  private setupInput(): void {
    if (!this.input.keyboard) return;

    this.cursors = this.input.keyboard.createCursorKeys();
    this.wasd = {
      W: this.input.keyboard.addKey(Phaser.Input.Keyboard.KeyCodes.W),
      A: this.input.keyboard.addKey(Phaser.Input.Keyboard.KeyCodes.A),
      S: this.input.keyboard.addKey(Phaser.Input.Keyboard.KeyCodes.S),
      D: this.input.keyboard.addKey(Phaser.Input.Keyboard.KeyCodes.D),
    };
    this.setupWindowKeyboardFallback();
  }

  private setupWindowKeyboardFallback(): void {
    if (typeof window === 'undefined') return;

    const shouldIgnoreKeyboardEvent = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (!target) return false;
      const tagName = target.tagName;
      return (
        tagName === 'INPUT' ||
        tagName === 'TEXTAREA' ||
        tagName === 'SELECT' ||
        target.isContentEditable
      );
    };

    const setKeyState = (event: KeyboardEvent, isDown: boolean) => {
      if (shouldIgnoreKeyboardEvent(event)) return;
      switch (event.code) {
        case 'KeyW':
          this.windowKeyState.W = isDown;
          break;
        case 'KeyA':
          this.windowKeyState.A = isDown;
          break;
        case 'KeyS':
          this.windowKeyState.S = isDown;
          break;
        case 'KeyD':
          this.windowKeyState.D = isDown;
          break;
        default:
          return;
      }
      event.preventDefault();
    };

    this.windowKeyDownHandler = (event) => setKeyState(event, true);
    this.windowKeyUpHandler = (event) => setKeyState(event, false);
    window.addEventListener('keydown', this.windowKeyDownHandler);
    window.addEventListener('keyup', this.windowKeyUpHandler);
  }

  private teardownInput(): void {
    if (typeof window === 'undefined') return;
    if (this.windowKeyDownHandler) {
      window.removeEventListener('keydown', this.windowKeyDownHandler);
      this.windowKeyDownHandler = null;
    }
    if (this.windowKeyUpHandler) {
      window.removeEventListener('keyup', this.windowKeyUpHandler);
      this.windowKeyUpHandler = null;
    }
    this.windowKeyState = { W: false, A: false, S: false, D: false };
  }

  private clearMovementInputState(): void {
    this.windowKeyState = { W: false, A: false, S: false, D: false };
    this.cursors?.left?.reset();
    this.cursors?.right?.reset();
    this.cursors?.up?.reset();
    this.cursors?.down?.reset();
    this.wasd?.W?.reset();
    this.wasd?.A?.reset();
    this.wasd?.S?.reset();
    this.wasd?.D?.reset();
  }

  private handlePlayerMovement(time: number, delta: number): void {
    if (!this.player || !this.input.keyboard) return;
    if (this.floorTransitionInFlight || this.time.now < this.playerMovementLockedUntil) {
      const pState = getStoreState().player;
      const idleDir = pState ? pState.direction : 'down';
      const animKey = `player_idle_${idleDir}`;
      if (this.anims.exists(animKey)) {
        this.player.play(animKey, true);
      }
      this.player.setOrigin(0.5, 0.75);
      if (this.playerShadow) {
        this.playerShadow.x = this.player.x;
        this.playerShadow.y = this.player.y + 2;
      }
      return;
    }

    let vx = 0;
    let vy = 0;

    const left = this.cursors.left.isDown || this.wasd.A.isDown || this.windowKeyState.A;
    const right = this.cursors.right.isDown || this.wasd.D.isDown || this.windowKeyState.D;
    const up = this.cursors.up.isDown || this.wasd.W.isDown || this.windowKeyState.W;
    const down = this.cursors.down.isDown || this.wasd.S.isDown || this.windowKeyState.S;

    if (left) vx -= 1;
    if (right) vx += 1;
    if (up) vy -= 1;
    if (down) vy += 1;

    // Normalize diagonal
    if (vx !== 0 && vy !== 0) {
      const len = Math.SQRT2;
      vx /= len;
      vy /= len;
    }

    const speed = this.playerSpeed * (delta / 1000);
    const nextX = this.player.x + vx * speed;
    const nextY = this.player.y + vy * speed;

    if (this.isWalkableAt(nextX, this.player.y)) {
      this.player.x = nextX;
    }
    if (this.isWalkableAt(this.player.x, nextY)) {
      this.player.y = nextY;
    }

    // Clamp to world bounds
    const bounds = this.physics.world.bounds;
    this.player.x = Phaser.Math.Clamp(this.player.x, SPRITE_PX / 2, bounds.width - SPRITE_PX / 2);
    this.player.y = Phaser.Math.Clamp(this.player.y, SPRITE_PX / 2, bounds.height - SPRITE_PX / 4);

    // Play directional animations
    const isMoving = vx !== 0 || vy !== 0;
    if (isMoving) {
      let dir = 'down';
      if (Math.abs(vx) > Math.abs(vy)) {
        dir = vx > 0 ? 'right' : 'left';
      } else {
        dir = vy > 0 ? 'down' : 'up';
      }
      const animKey = `player_walk_${dir}`;
      if (this.anims.exists(animKey)) {
        this.player.play(animKey, true);
      }
      const pState = getStoreState().player;
      if (pState) {
        pState.direction = dir as 'up' | 'down' | 'left' | 'right';
      }

      // Add walk bobbing juice
      const bobOffset = Math.sin(time * 0.015) * 1.5;
      this.player.setOrigin(0.5, 0.75 + bobOffset * 0.01);

      // Kick up dust
      if (Math.random() < 0.2) {
        this.dustEmitter.emitParticleAt(this.player.x, this.player.y + 4);
      }
    } else {
      this.player.setOrigin(0.5, 0.75); // Reset origin

      // Use last facing direction for idle (read from store)
      const pState = getStoreState().player;
      const idleDir = pState ? pState.direction : 'down';
      const animKey = `player_idle_${idleDir}`;
      if (this.anims.exists(animKey)) {
        this.player.play(animKey, true);
      }
    }

    // Sync player position back to the Zustand store
    const store = getStoreState();
    if (store.player) {
      // Lightweight: only update x/y without triggering full set()
      store.player.x = this.sceneToWorldX(this.player.x);
      store.player.y = this.sceneToWorldY(this.player.y);
    }

    // Check room entry
    this.checkPlayerRoom();
    this.maybeTriggerFloorTransition();

    // Sync shadow position
    if (this.playerShadow) {
      this.playerShadow.x = this.player.x;
      this.playerShadow.y = this.player.y + 2; 
    }
  }

  private checkPlayerRoom(): void {
    if (this.maskPixels) return;
    const state = getStoreState();
    let foundRoom: string | null = null;

    for (const house of state.houses.values()) {
      const rx = house.room.tileX * TILE_SIZE;
      const ry = house.room.tileY * TILE_SIZE;
      const rw = house.room.width * TILE_SIZE;
      const rh = house.room.height * TILE_SIZE;

      if (
        this.player.x >= rx && this.player.x <= rx + rw &&
        this.player.y >= ry && this.player.y <= ry + rh
      ) {
        foundRoom = house.id;
        break;
      }
    }

    if (foundRoom !== this.activeRoomId) {
      this.setActiveRoom(foundRoom);
      if (foundRoom) {
        EventBus.emit(EVT_ROOM_ENTERED, foundRoom);
      }
    }
  }

  // =========================================================================
  // Camera
  // =========================================================================

  private setupCamera(): void {
    const cam = this.cameras.main;
    cam.setRoundPixels(true);
    cam.setZoom(CAMERA_ZOOM);

    if (this.player) {
      cam.startFollow(this.player, true, CAMERA_LERP, CAMERA_LERP);
    }
  }

  private setupCameraDrag(): void {
    const DRAG_THRESHOLD_PX = 6;

    this.input.on('pointerdown', (pointer: Phaser.Input.Pointer) => {
      if (pointer.button !== 0) return;
      this.dragPointerId = pointer.id;
      this.isCameraDragging = false;
      this.dragStartScreenX = pointer.x;
      this.dragStartScreenY = pointer.y;
      this.dragStartScrollX = this.cameras.main.scrollX;
      this.dragStartScrollY = this.cameras.main.scrollY;
    });

    this.input.on('pointermove', (pointer: Phaser.Input.Pointer) => {
      if (this.dragPointerId !== pointer.id || !pointer.isDown) return;

      const dx = pointer.x - this.dragStartScreenX;
      const dy = pointer.y - this.dragStartScreenY;

      if (!this.isCameraDragging) {
        if (Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return;
        this.isCameraDragging = true;
        this.suppressSelectionUntil = this.time.now + 200;
        this.cameras.main.stopFollow();
      }

      const cam = this.cameras.main;
      const bounds = this.physics.world.bounds;
      const viewWidth = cam.width / cam.zoom;
      const viewHeight = cam.height / cam.zoom;
      const nextScrollX = this.dragStartScrollX - dx / cam.zoom;
      const nextScrollY = this.dragStartScrollY - dy / cam.zoom;

      cam.scrollX = Phaser.Math.Clamp(nextScrollX, 0, Math.max(0, bounds.width - viewWidth));
      cam.scrollY = Phaser.Math.Clamp(nextScrollY, 0, Math.max(0, bounds.height - viewHeight));
    });

    this.input.on('pointerup', (pointer: Phaser.Input.Pointer) => {
      if (this.dragPointerId !== pointer.id) return;
      if (this.isCameraDragging) {
        this.suppressSelectionUntil = this.time.now + 200;
        // Resume camera follow after drag ends
        if (this.player) {
          this.cameras.main.startFollow(this.player, true, CAMERA_LERP, CAMERA_LERP);
        }
      }
      this.isCameraDragging = false;
      this.dragPointerId = null;
    });
  }

  private shouldSuppressSelection(): boolean {
    return this.isCameraDragging || this.time.now < this.suppressSelectionUntil;
  }

  // =========================================================================
  // NPC Agents
  // =========================================================================

  private spawnAgents(
    agents: Map<string, PixelAgentState>,
  ): void {
    // spawnAgents: processing ${agents.size} agents
    let spawned = 0;
    for (const [id, agent] of agents) {
      if (id === 'player') continue;
      const agentFloorId: FloorConfig['id'] = 'floor1';
      this.agentFloorAssignments.set(id, agentFloorId);

      let textureKey: string;
      try {
        textureKey = loadAgentTexture(this, id, agent.sprite);
        createAgentAnimations(this, id, agent.sprite);
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        pixEventMs('pixel_asset_load_error', {
          asset_key: `agent-${id}`,
          agent_id: id,
          stage: 'load_agent_texture',
          error: msg,
        });
        console.error(`[MainScene] Failed to load agent texture for ${id}:`, err);
        // Fallback texture
        if (!this.textures.exists('npc-fallback')) {
          const gfx = this.add.graphics();
          gfx.fillStyle(0x6688cc, 1);
          gfx.fillRect(0, 0, SPRITE_PX, SPRITE_PX);
          gfx.generateTexture('npc-fallback', SPRITE_PX, SPRITE_PX);
          gfx.destroy();
        }
        textureKey = 'npc-fallback';
      }

      const startX = this.worldToSceneX(agent.anim.x);
      const startY = this.worldToSceneY(agent.anim.y);

      const npcInitialFrame = textureKey === 'agent-atlas' ? `${id}_0` : 0;
      const sprite = this.add.sprite(startX, startY, textureKey, npcInitialFrame);
      sprite.setScale(1.2);
      sprite.setOrigin(0.5, 0.75);
      sprite.setInteractive({ useHandCursor: true });

      sprite.on('pointerup', () => {
        if (this.shouldSuppressSelection()) return;
        // Emit a SINGLE click event. Resolve `room_id` from the LIVE store
        // state at click time — closing over the spawn-time `agent` would
        // report a stale room after the agent walks to a new one (the
        // store replaces the agent object immutably on every tick, so the
        // closure's copy is forever anchored to the spawn position).
        const liveAgent = usePixelWorldStore.getState().agents.get(id);
        EventBus.emit(EVT_AGENT_CLICKED, {
          agent_id: id,
          room_id: liveAgent?.currentHouseId ?? null,
        });
      });
      this.agentSprites.set(id, sprite);

      const shadow = this.add.ellipse(startX, startY, 14, 6, 0x000000, 0.25);
      shadow.setDepth(-8.5);
      this.agentShadows.set(id, shadow);

      // Name label. The scene runs with pixelArt: true (global NEAREST
      // filter) and CAMERA_ZOOM=0.82, so a Text canvas gets non-integer
      // downsampling that NEAREST mangles. Match the canvas to display-pixel
      // resolution and force LINEAR filtering on the label texture so the
      // glyphs stay legible while the camera moves.
      const dpr = typeof window !== 'undefined' ? (window.devicePixelRatio || 1) : 1;
      const label = this.add.text(startX, startY - SPRITE_PX, agent.name, {
        fontSize: '12px',
        fontFamily: '"Press Start 2P", "Silkscreen", "VT323", "Courier New", monospace',
        color: '#ffffff',
        backgroundColor: '#00000088',
        padding: { x: 3, y: 2 },
        resolution: Math.max(2, dpr / CAMERA_ZOOM),
      });
      label.setOrigin(0.5, 1);
      label.setDepth(9999);
      label.texture.setFilter(Phaser.Textures.FilterMode.LINEAR);
      this.agentNameLabels.set(id, label);

      // Bubble
      const bubble = this.add.text(startX, startY - SPRITE_PX, '', {
        fontSize: '11px',
        resolution: 2,
      });
      bubble.setOrigin(0.5, 1);
      bubble.setDepth(10000);
      bubble.setAlpha(0);
      this.agentBubbles.set(id, bubble);

      // Play idle animation
      const idleKey = `${id}_idle_down`;
      if (this.anims.exists(idleKey)) {
        sprite.play(idleKey, true);
      }
      this.applyAgentVisibility(id, agentFloorId === this.currentFloorId);
      spawned++;
    }
    // NPC sprite spawning complete
  }

  private applyAgentVisibility(agentId: string, visible: boolean): void {
    const sprite = this.agentSprites.get(agentId);
    if (sprite) {
      sprite.setVisible(visible);
      sprite.disableInteractive();
      if (visible) {
        sprite.setInteractive({ useHandCursor: true });
      }
    }

    const shadow = this.agentShadows.get(agentId);
    if (shadow) shadow.setVisible(visible);

    const label = this.agentNameLabels.get(agentId);
    if (label) label.setVisible(visible);

    const bubble = this.agentBubbles.get(agentId);
    if (bubble) bubble.setVisible(visible);
  }

  /**
   * Sync NPC sprite positions from the Zustand store's agent anim state.
   * The store's tick() still runs agent AI (walk targets, state transitions).
   * We just mirror those positions into the Phaser sprites.
   */
  private syncNPCPositions(time: number): void {
    const state = getStoreState();

    for (const [id, agent] of state.agents) {
      if (id === 'player') continue;

      const sprite = this.agentSprites.get(id);
      if (!sprite) continue;
      const agentFloorId = this.agentFloorAssignments.get(id) ?? 'floor1';
      const isVisibleOnCurrentFloor = agentFloorId === this.currentFloorId;
      this.applyAgentVisibility(id, isVisibleOnCurrentFloor);
      if (!isVisibleOnCurrentFloor) continue;

      const anim = agent.anim;

      // Smooth lerp toward store position
      const targetX = this.worldToSceneX(anim.x);
      const targetY = this.worldToSceneY(anim.y);
      sprite.x += (targetX - sprite.x) * 0.15;
      sprite.y += (targetY - sprite.y) * 0.15;

      // Update animation based on agent state.
      //
      // Directional states (walk/work/think/idle) use `${id}_${state}_${dir}`.
      // Status states (typing/thinking/sitting/talking) are front-only and
      // use `${id}_${state}`. Status anim keys are ONLY registered on
      // Choruz roster agents (192×384 sheet with status rows 4-7); legacy
      // atlas agents don't have them and fall back to idle in the current
      // direction via the `fallbackAnimKey` path below.
      const dir = anim.direction;
      const animKey = isStatusAnimState(anim.state)
        ? `${id}_${anim.state}`
        : `${id}_${anim.state}_${dir}`;

      // Shared guard: (re)start the anim when it's not currently playing,
      // or when it IS playing but a different key was requested. The
      // `!isPlaying` clause is important because `sprite.anims.stop()` in
      // the tier-3 fallback below leaves `currentAnim` set — without the
      // isPlaying check, a subsequent request for that same key would be
      // suppressed by the key-inequality guard and leave the sprite frozen.
      const needsPlay = (key: string): boolean =>
        sprite.anims.currentAnim?.key !== key || !sprite.anims.isPlaying;

      if (this.anims.exists(animKey)) {
        if (needsPlay(animKey)) sprite.play(animKey, true);
      } else {
        // Requested anim not registered for this agent (commonly: a legacy
        // atlas agent entered a status state that only exists for roster
        // agents). Play idle in the current direction instead, so the
        // sprite doesn't freeze on a stale walk/work frame.
        const fallbackAnimKey = `${id}_idle_${dir}`;
        if (this.anims.exists(fallbackAnimKey)) {
          if (needsPlay(fallbackAnimKey)) sprite.play(fallbackAnimKey, true);
        } else {
          // Tier-3 fallback: even idle isn't registered. Stop any stale
          // animation first, otherwise a leftover walk would keep playing
          // despite the state being out-of-sync. Then set a static base
          // frame that's correct for whichever texture backs this sprite:
          //   - atlas-backed agents use namespaced frame name `${id}_0`
          //   - per-agent textures (roster + procedural) use numeric 0
          if (sprite.anims.isPlaying) sprite.anims.stop();
          const atlasFrameName = `${id}_0`;
          const tex = sprite.texture;
          if (tex.has(atlasFrameName)) {
            sprite.setFrame(atlasFrameName);
          } else if (tex.has('0')) {
            sprite.setFrame(0);
          }
          // If neither frame exists the sprite keeps its current (stopped)
          // frame, which is still preferable to stale motion.
        }
      }

      if (anim.state === 'walk') {
        // Add desynced bobbing juice
        const bobOffset = Math.sin(time * 0.015 + sprite.x * 0.1) * 1.5;
        sprite.setOrigin(0.5, 0.75 + bobOffset * 0.01);
        
        // Kick up dust
        if (Math.random() < 0.1) {
          this.dustEmitter.emitParticleAt(sprite.x, sprite.y + 4);
        }
      } else {
        sprite.setOrigin(0.5, 0.75);
      }

      // Sync shadow
      const shadow = this.agentShadows.get(id);
      if (shadow) {
        shadow.x = sprite.x;
        shadow.y = sprite.y + 2;
      }

      // Update label position. Round to integers — Text objects are
      // sampled from an off-screen canvas, so sub-pixel positions blur the
      // glyphs even when the camera has roundPixels enabled.
      const label = this.agentNameLabels.get(id);
      if (label) {
        label.x = Math.round(sprite.x);
        label.y = Math.round(sprite.y - SPRITE_PX);
      }

      // Sync bubble
      const bubble = this.agentBubbles.get(id);
      if (bubble) {
        if (anim.bubble) {
          bubble.setText(anim.bubble);
          bubble.x = sprite.x + 8;
          bubble.y = sprite.y - SPRITE_PX * 0.5 - ((60 - (anim.bubbleTicks ?? 0)) % 60) * 0.1; // Gentle float
          
          if (bubble.alpha === 0) {
            bubble.setAlpha(1);
            bubble.setScale(0);
            this.tweens.add({
              targets: bubble,
              scale: 1,
              duration: 200,
              ease: 'Back.easeOut',
            });
          }
        } else if (bubble.alpha > 0) {
          this.tweens.add({
            targets: bubble,
            alpha: 0,
            scale: 0.5,
            duration: 150,
            ease: 'Power2',
            onComplete: () => bubble.setText(''),
          });
        }
      }
    }
  }

  /**
   * Periodically trigger NPC wander in the store.
   * The store already has walk/idle state transitions;
   * we just nudge idle agents to walk somewhere new.
   */
  private setupNPCWander(): void {
    this.wanderTimer = this.time.addEvent({
      delay: NPC_WANDER_INTERVAL,
      loop: true,
      callback: () => {
        const state = getStoreState();
        for (const [id, agent] of state.agents) {
          if (id === 'player') continue;
          if (agent.anim.state !== 'idle') continue;
          if (Math.random() > 0.3) continue; // only 30% chance to wander

          // Ambient wander within the agent's current room. Uses the
          // dedicated store action so `handleMessage`'s "sender just sent"
          // semantics (talking → typing) don't fire for a mere wander.
          state.wanderAgentInRoom(id);
        }
      },
    });
  }

  // =========================================================================
  // Y-Sort
  // =========================================================================

  private ySortDepth(): void {
    if (this.player) {
      this.player.setDepth(this.player.y + 10);
    }
    this.agentSprites.forEach((sprite) => {
      sprite.setDepth(sprite.y + 10);
    });
  }

  // =========================================================================
  // EventBus listeners (React -> Phaser)
  // =========================================================================

  private setupEventListeners(): void {
    EventBus.on(EVT_FOCUS_AGENT, (agentId: string) => {
      const sprite = this.agentSprites.get(agentId);
      if (sprite) {
        this.cameras.main.pan(sprite.x, sprite.y, 800, 'Power2');
      }
    });

    EventBus.on(EVT_HIGHLIGHT_ROOM, (roomId: string) => {
      this.setActiveRoom(roomId);
      // In the custom floor-background mode, old logical houses are not
      // visually meaningful screen anchors, so do not pan the camera here.
      if (this.maskPixels) return;

      // Also pan camera to room center
      const state = getStoreState();
      const house = state.houses.get(roomId);
      if (house) {
        const cx = this.worldToSceneX(house.room.tileX * TILE_SIZE + (house.room.width * TILE_SIZE) / 2);
        const cy = this.worldToSceneY(house.room.tileY * TILE_SIZE + (house.room.height * TILE_SIZE) / 2);
        this.cameras.main.pan(cx, cy, 800, 'Power2');
      }
    });

    EventBus.on(EVT_WORLD_DATA_CHANGED, () => {
      this.rebuildWorld();
    });
  }

  // =========================================================================
  // Store sync — detect structural changes requiring world rebuild
  // =========================================================================

  private _rebuilding = false;

  private setupStoreSync(): void {
    this.unsubscribe = subscribeStore((state) => {
      if (this._rebuilding) return; // prevent re-entrant rebuild loop

      const hasData = state.tileGrid !== null && state.houses.size > 0;
      const structureChanged = state.agents.size !== this.lastAgentCount ||
          state.houses.size !== this.lastHouseCount;

      if (hasData && (structureChanged || this.lastHouseCount === 0)) {
        this.lastAgentCount = state.agents.size;
        this.lastHouseCount = state.houses.size;
        this._rebuilding = true;
        this.time.delayedCall(200, () => {
          this.rebuildWorld();
          this._rebuilding = false;
        });
      }
    });
  }
}
