export type TerminalChunk = string | Uint8Array;

const DEFAULT_FRAME_BUDGET = 256 * 1024;
const DEFAULT_MAX_PENDING = 8 * 1024 * 1024;

/**
 * Coalesces WebSocket frames into bounded, animation-frame-sized xterm writes.
 * This keeps terminal bursts from monopolising the main thread while retaining
 * character order. If a producer outruns the browser beyond the pending limit,
 * the oldest queued output is discarded and an explicit marker is rendered.
 */
export function createTerminalWriteBuffer(
  write: (data: string) => void,
  schedule: (callback: FrameRequestCallback) => number = requestAnimationFrame,
  cancel: (handle: number) => void = cancelAnimationFrame,
  frameBudget = DEFAULT_FRAME_BUDGET,
  maxPendingCharacters = DEFAULT_MAX_PENDING,
) {
  const decoder = new TextDecoder();
  let pending = "";
  let scheduled: number | null = null;
  let disposed = false;
  let dropped = false;

  const flush = () => {
    scheduled = null;
    if (disposed || pending.length === 0) return;
    const chunk = pending.slice(0, frameBudget);
    pending = pending.slice(chunk.length);
    write((dropped ? "\r\n\x1b[33m[terminal output truncated during burst]\x1b[0m\r\n" : "") + chunk);
    dropped = false;
    if (pending.length > 0) scheduled = schedule(flush);
  };

  const enqueue = (chunk: TerminalChunk) => {
    if (disposed) return;
    if (typeof chunk === "string") {
      pending += decoder.decode();
      pending += chunk;
    } else {
      pending += decoder.decode(chunk, { stream: true });
    }
    if (pending.length > maxPendingCharacters) {
      pending = pending.slice(-maxPendingCharacters);
      dropped = true;
    }
    if (scheduled === null) scheduled = schedule(flush);
  };

  return {
    enqueue,
    pendingCharacters: () => pending.length,
    dispose: () => {
      disposed = true;
      pending = "";
      if (scheduled !== null) cancel(scheduled);
      scheduled = null;
    },
  };
}
