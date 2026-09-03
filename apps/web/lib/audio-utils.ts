/**
 * Audio utilities for recording and encoding WAV files.
 *
 * Uses only native Web APIs — no npm packages required.
 */

/**
 * Encode raw f32 PCM samples as a WAV file (32-bit float, mono).
 *
 * Returns an ArrayBuffer ready to POST to the server.
 */
export function encodeWav(
  samples: Float32Array,
  sampleRate: number,
): ArrayBuffer {
  const numChannels = 1;
  const bitsPerSample = 32;
  const bytesPerSample = bitsPerSample / 8;
  const blockAlign = numChannels * bytesPerSample;
  const dataSize = samples.length * bytesPerSample;

  // WAV header is 44 bytes
  const buffer = new ArrayBuffer(44 + dataSize);
  const view = new DataView(buffer);

  // RIFF header
  writeString(view, 0, "RIFF");
  view.setUint32(4, 36 + dataSize, true);
  writeString(view, 8, "WAVE");

  // fmt chunk
  writeString(view, 12, "fmt ");
  view.setUint32(16, 16, true); // chunk size
  view.setUint16(20, 3, true); // format = IEEE float
  view.setUint16(22, numChannels, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * blockAlign, true); // byte rate
  view.setUint16(32, blockAlign, true);
  view.setUint16(34, bitsPerSample, true);

  // data chunk
  writeString(view, 36, "data");
  view.setUint32(40, dataSize, true);

  // Write samples
  const offset = 44;
  for (let i = 0; i < samples.length; i++) {
    view.setFloat32(offset + i * 4, samples[i], true);
  }

  return buffer;
}

function writeString(view: DataView, offset: number, str: string): void {
  for (let i = 0; i < str.length; i++) {
    view.setUint8(offset + i, str.charCodeAt(i));
  }
}

/**
 * Resample an AudioBuffer to 16 kHz mono using OfflineAudioContext.
 */
export async function resampleTo16k(
  buffer: AudioBuffer,
): Promise<Float32Array> {
  const targetRate = 16000;
  const duration = buffer.duration;
  const outLength = Math.ceil(duration * targetRate);

  const offlineCtx = new OfflineAudioContext(1, outLength, targetRate);
  const source = offlineCtx.createBufferSource();
  source.buffer = buffer;
  source.connect(offlineCtx.destination);
  source.start(0);

  const rendered = await offlineCtx.startRendering();
  return rendered.getChannelData(0);
}
