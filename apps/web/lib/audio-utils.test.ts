import { describe, expect, it } from "vitest";
import { encodeWav } from "./audio-utils";

// Helpers ---------------------------------------------------------------------

function readAscii(buf: ArrayBuffer, offset: number, len: number): string {
  const view = new DataView(buf);
  let s = "";
  for (let i = 0; i < len; i++) s += String.fromCharCode(view.getUint8(offset + i));
  return s;
}

// encodeWav -------------------------------------------------------------------

describe("encodeWav", () => {
  it("emits an output sized 44-byte header + 4-byte-per-sample data", () => {
    const samples = new Float32Array(100);
    const buf = encodeWav(samples, 44100);
    expect(buf.byteLength).toBe(44 + 100 * 4);
  });

  it("writes the RIFF / WAVE / fmt / data magic markers in the right slots", () => {
    const buf = encodeWav(new Float32Array(8), 16000);
    expect(readAscii(buf, 0, 4)).toBe("RIFF");
    expect(readAscii(buf, 8, 4)).toBe("WAVE");
    expect(readAscii(buf, 12, 4)).toBe("fmt ");
    expect(readAscii(buf, 36, 4)).toBe("data");
  });

  it("encodes IEEE float (format=3), mono, 32-bit per sample header fields", () => {
    const sampleRate = 22050;
    const buf = encodeWav(new Float32Array(4), sampleRate);
    const view = new DataView(buf);
    // chunk size at offset 16 = 16
    expect(view.getUint32(16, true)).toBe(16);
    // format at offset 20 = 3 (IEEE float)
    expect(view.getUint16(20, true)).toBe(3);
    // numChannels at offset 22 = 1 (mono)
    expect(view.getUint16(22, true)).toBe(1);
    // sampleRate at offset 24
    expect(view.getUint32(24, true)).toBe(sampleRate);
    // byteRate at offset 28 = sampleRate * 4
    expect(view.getUint32(28, true)).toBe(sampleRate * 4);
    // blockAlign at offset 32 = 4
    expect(view.getUint16(32, true)).toBe(4);
    // bitsPerSample at offset 34 = 32
    expect(view.getUint16(34, true)).toBe(32);
  });

  it("writes correct dataSize (offset 40) and RIFF chunk size (offset 4)", () => {
    const samples = new Float32Array(50);
    const buf = encodeWav(samples, 8000);
    const view = new DataView(buf);
    expect(view.getUint32(4, true)).toBe(36 + 50 * 4);
    expect(view.getUint32(40, true)).toBe(50 * 4);
  });

  it("preserves sample values (little-endian f32 round-trip)", () => {
    const samples = new Float32Array([0, 0.5, -0.5, 1, -1]);
    const buf = encodeWav(samples, 16000);
    const view = new DataView(buf);
    for (let i = 0; i < samples.length; i++) {
      expect(view.getFloat32(44 + i * 4, true)).toBeCloseTo(samples[i], 5);
    }
  });

  it("handles zero-length input (header only, no data)", () => {
    const buf = encodeWav(new Float32Array(0), 44100);
    expect(buf.byteLength).toBe(44);
    const view = new DataView(buf);
    expect(view.getUint32(40, true)).toBe(0); // dataSize == 0
  });
});
