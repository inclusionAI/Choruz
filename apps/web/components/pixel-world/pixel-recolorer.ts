/**
 * apps/web/components/pixel-world/pixel-recolorer.ts
 *
 * Industrial Grade Pixel Recolorer (Palette Swapping Engine)
 * This engine takes a pristine AI-generated baseline sprite sheet, scans every pixel
 * via Canvas getImageData, and shifts the primary HSL/RGB colors dynamically.
 */

// Utility: converts hex to RGB
export function hexToRgb(hex: string): { r: number; g: number; b: number } {
  const result = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})$/i.exec(hex);
  return result
    ? {
        r: parseInt(result[1], 16),
        g: parseInt(result[2], 16),
        b: parseInt(result[3], 16),
      }
    : { r: 0, g: 0, b: 0 };
}

// Utility: simple RGB to HSL
export function rgbToHsl(r: number, g: number, b: number) {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  let h = 0, s = 0, l = (max + min) / 2;

  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    switch (max) {
      case r: h = (g - b) / d + (g < b ? 6 : 0); break;
      case g: h = (b - r) / d + 2; break;
      case b: h = (r - g) / d + 4; break;
    }
    h /= 6;
  }
  return [h, s, l];
}

// Utility: HSL to RGB
export function hslToRgb(h: number, s: number, l: number) {
  let r, g, b;

  if (s === 0) {
    r = g = b = l; // achromatic
  } else {
    const hue2rgb = (p: number, q: number, t: number) => {
      if (t < 0) t += 1;
      if (t > 1) t -= 1;
      if (t < 1 / 6) return p + (q - p) * 6 * t;
      if (t < 1 / 2) return q;
      if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
      return p;
    };
    const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
    const p = 2 * l - q;
    r = hue2rgb(p, q, h + 1 / 3);
    g = hue2rgb(p, q, h);
    b = hue2rgb(p, q, h - 1 / 3);
  }
  return [Math.round(r * 255), Math.round(g * 255), Math.round(b * 255)];
}

/**
 * Recolor Engine
 * Scans an Image, looks for pixels that have strong saturation (like a primary colored shirt),
 * and rotates their Hue to match the target color, while preserving the exact Lightness.
 */
export function recolorSprite(
  sourceImg: HTMLImageElement | HTMLCanvasElement,
  targetHexColor: string,
  keyHueRange: [number, number] = [0, 1] // Which Hues to replace (0.0 to 1.0). By default we shift everything saturated.
): HTMLCanvasElement {
  
  const width = sourceImg.width;
  const height = sourceImg.height;

  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext('2d')!;

  // Draw original
  ctx.drawImage(sourceImg, 0, 0);

  // Get raw pixels
  const imageData = ctx.getImageData(0, 0, width, height);
  const data = imageData.data;

  // Determine target hue
  const targetRgb = hexToRgb(targetHexColor);
  const [targetH, targetS, ] = rgbToHsl(targetRgb.r, targetRgb.g, targetRgb.b);

  for (let i = 0; i < data.length; i += 4) {
    const r = data[i];
    const g = data[i + 1];
    const b = data[i + 2];
    const a = data[i + 3];

    // Ignore fully transparent pixels
    if (a < 10) continue;

    // Convert pixel to HSL
    const [h, s, l] = rgbToHsl(r, g, b);

    // If the pixel is notably colored (not grayscale skin/shadow/outline)
    // AND it falls within our targeted hue range (if we specified one)
    if (s > 0.15 && h >= keyHueRange[0] && h <= keyHueRange[1]) {
      
      // Magic: We keep original Lightness (l), but inject Target Hue & Saturation
      const originalLightnessPreserved = l; 
      
      // Combine it back
      const [newR, newG, newB] = hslToRgb(targetH, targetS, originalLightnessPreserved);

      data[i] = newR;
      data[i + 1] = newG;
      data[i + 2] = newB;
    }
  }

  // Put colored pixels back
  ctx.putImageData(imageData, 0, 0);
  
  return canvas;
}
