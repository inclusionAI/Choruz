// config.ts — Phaser 3 game configuration for the pixel-world office.
// WebGL renderer with pixel-art settings, RESIZE scaling, top-down arcade physics.

import * as Phaser from 'phaser';
import { MainScene } from './scenes/main-scene';

export function createGameConfig(parent: string | HTMLElement): Phaser.Types.Core.GameConfig {
  return {
    // Chrome / WebGL path is currently unreliable for our runtime-generated textures.
    // Force Canvas so character and tile textures render consistently.
    type: Phaser.CANVAS,
    parent,
    // Initial dimensions — RESIZE mode will adapt to container automatically.
    width: 800,
    height: 600,
    pixelArt: true,
    roundPixels: true,
    backgroundColor: '#0f0f14',
    scale: {
      mode: Phaser.Scale.RESIZE,
      autoCenter: Phaser.Scale.CENTER_BOTH,
    },
    physics: {
      default: 'arcade',
      arcade: {
        gravity: { x: 0, y: 0 },
        debug: false,
      },
    },
    scene: [MainScene],
    // Let Phaser capture keyboard events on its own canvas so WASD movement works.
    // The canvas element is focused when the user clicks the Pixel World panel.
  };
}
