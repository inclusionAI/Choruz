import { sanitizeTelemetryData } from "./telemetry-sanitize";

// choruz-trace.ts — Unified front-end tracing that logs to console AND sends
// events to the gateway so they appear in the same log stream as back-end spans.
//
// Usage:
//   const span = trace.start("select_conversation", { convId });
//   ... do work ...
//   span.end({ messageCount: 42 });          // auto-calculates duration
//   span.end({ error: "not found" });         // mark as error
//
//   trace.event("ws_connected", { url });     // one-shot event (no duration)

let _sessionToken: string | null = null;
let _gatewayBase = "";

/** Call once at app startup to wire up the telemetry endpoint. */
export function initTrace(sessionToken: string, gatewayBase: string) {
  _sessionToken = sessionToken;
  _gatewayBase = gatewayBase;
}

// ---------------------------------------------------------------------------
// Trace ID — one per user action chain (click → API → render)
// ---------------------------------------------------------------------------

let _activeTraceId: string | null = null;
/** Handle for the "idle clear" timer that evicts `_activeTraceId` after the
 * user action quiesces. Without this, a stale id can bleed into unrelated
 * later events and produce false joins (round-3 P1 finding). */
let _activeTraceTimer: ReturnType<typeof setTimeout> | null = null;
/** How long an active trace id survives without fresh activity before it
 * gets cleared. 10s comfortably covers a click → API → render chain on a
 * slow network but still prevents two unrelated clicks from merging. */
const ACTIVE_TRACE_TTL_MS = 10_000;

function newTraceId(): string {
  // 8-char hex, cheap & unique enough for local debugging
  return Math.random().toString(16).slice(2, 10);
}

function scheduleTraceExpiry(): void {
  if (_activeTraceTimer) clearTimeout(_activeTraceTimer);
  _activeTraceTimer = setTimeout(() => {
    _activeTraceId = null;
    _activeTraceTimer = null;
  }, ACTIVE_TRACE_TTL_MS);
}

/** Start a new trace chain (call at user-action entry points). */
export function beginTrace(): string {
  _activeTraceId = newTraceId();
  scheduleTraceExpiry();
  return _activeTraceId;
}

/** Explicitly end the current action — clears the active trace id now so
 * the next event in a new action gets a fresh id instead of joining onto
 * a stale one. Safe to call multiple times.
 *
 * Pass `expectedId` to make the clear conditional: only clears if the
 * still-active id matches the caller's own trace. This prevents an
 * in-flight action A from stomping a subsequent action B's trace when
 * A finishes (race: A starts → B starts → A ends → B's trace cleared).
 * Callers that are sure they own the active trace can call without args. */
export function endTrace(expectedId?: string): void {
  if (expectedId !== undefined && _activeTraceId !== expectedId) {
    // Someone else took over the active trace (e.g. user started a new
    // action). Leave it alone.
    return;
  }
  _activeTraceId = null;
  if (_activeTraceTimer) {
    clearTimeout(_activeTraceTimer);
    _activeTraceTimer = null;
  }
}

export function currentTraceId(): string | null {
  return _activeTraceId;
}

// ---------------------------------------------------------------------------
// Span — a timed operation
// ---------------------------------------------------------------------------

export interface Span {
  /** End the span. Pass extra data merged into the log entry. */
  end: (extra?: Record<string, unknown>) => void;
  /** The trace ID for this span (pass to API headers). */
  traceId: string;
}

export interface TraceEntry {
  source: "FE";
  traceId: string;
  spanId: string;
  name: string;
  ts: string;            // ISO timestamp
  durationMs?: number;   // set on span.end()
  data?: Record<string, unknown>;
}

function spanId(): string {
  return Math.random().toString(16).slice(2, 10);
}

// ---------------------------------------------------------------------------
// Ring buffer — keep last N entries in memory for in-page inspection
// ---------------------------------------------------------------------------

const RING_SIZE = 500;
const _ring: TraceEntry[] = [];

/** Access the in-memory log ring (newest last). */
export function traceRing(): readonly TraceEntry[] {
  return _ring;
}

function pushRing(entry: TraceEntry) {
  if (_ring.length >= RING_SIZE) _ring.shift();
  _ring.push(entry);
}

// ---------------------------------------------------------------------------
// Send to gateway (fire-and-forget)
// ---------------------------------------------------------------------------

const _queue: TraceEntry[] = [];
let _flushTimer: ReturnType<typeof setTimeout> | null = null;
const FLUSH_INTERVAL = 2000; // batch every 2s

function enqueue(entry: TraceEntry) {
  _queue.push(entry);
  if (!_flushTimer) {
    _flushTimer = setTimeout(flush, FLUSH_INTERVAL);
  }
}

function flush() {
  _flushTimer = null;
  if (_queue.length === 0 || !_sessionToken) return;
  const batch = _queue.splice(0);
  // Always use /api proxy (same origin as frontend) to avoid CORS issues.
  const url = "/api/v1/telemetry";
  fetch(url, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${_sessionToken}`,
    },
    body: JSON.stringify({ events: batch }),
  }).catch((err) => {
    // Log telemetry delivery failure to console (can't send a trace about trace failure)
    console.warn("[choruz-trace] telemetry flush failed:", err);
  });
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export const trace = {
  /** Start a timed span. Call span.end() when done. */
  start(name: string, data?: Record<string, unknown>): Span {
    // If there's no active trace we *start one* instead of emitting an
    // orphan span id — otherwise every span without a preceding click
    // would get its own isolated trace, breaking "group one user action".
    //
    // Only schedule the expiry on first auto-begin. Subsequent spans
    // within the same trace do NOT extend it — otherwise background
    // activity (pixel-world animation events, WS frames) could keep a
    // trace alive forever and the NEXT user click would inherit the
    // stale id. The 10s TTL fires once from the first auto-begin and
    // that's it.
    let tid = _activeTraceId;
    if (!tid) {
      tid = newTraceId();
      _activeTraceId = tid;
      scheduleTraceExpiry();
    }
    const sid = spanId();
    const t0 = performance.now();
    const ts = new Date().toISOString();

    const entry: TraceEntry = { source: "FE", traceId: tid, spanId: sid, name, ts, data: sanitizeTelemetryData(data) };

    // Log start immediately
    console.debug(`[choruz] ▶ ${name}`, sanitizeTelemetryData(data) ?? "", `trace=${tid}`);

    return {
      traceId: tid,
      end(extra?: Record<string, unknown>) {
        const durationMs = Math.round(performance.now() - t0);
        const merged = sanitizeTelemetryData({ ...data, ...extra }) ?? {};
        const final_entry: TraceEntry = {
          ...entry,
          durationMs,
          data: Object.keys(merged).length > 0 ? merged : undefined,
        };
        pushRing(final_entry);
        enqueue(final_entry);

        const tag = extra?.error ? "✗" : "✓";
        console.debug(
          `[choruz] ${tag} ${name} ${durationMs}ms`,
          merged,
          `trace=${tid}`,
        );
      },
    };
  },

  /** Log a one-shot event (no duration). */
  event(name: string, data?: Record<string, unknown>) {
    // Same lifecycle rule as `start`: schedule the expiry timer only when
    // auto-creating a fresh trace. Background events must never extend
    // the TTL — otherwise an unrelated later click would inherit this id.
    let tid = _activeTraceId;
    if (!tid) {
      tid = newTraceId();
      _activeTraceId = tid;
      scheduleTraceExpiry();
    }
    const ts = new Date().toISOString();
    const entry: TraceEntry = {
      source: "FE",
      traceId: tid,
      spanId: spanId(),
      name,
      ts,
      data: sanitizeTelemetryData(data),
    };
    pushRing(entry);
    enqueue(entry);
    console.debug(`[choruz] • ${name}`, sanitizeTelemetryData(data) ?? "", `trace=${tid}`);
  },
};

// ---------------------------------------------------------------------------
// Global user interaction tracking — logs every click/keypress to telemetry
// ---------------------------------------------------------------------------

let _interactionListenersAttached = false;

/** Call once after initTrace to start logging all user interactions. */
export function startInteractionTracking() {
  if (_interactionListenersAttached || typeof window === "undefined") return;
  _interactionListenersAttached = true;

  // Click tracking — walk up to nearest actionable element and report its
  // label. Every click starts a FRESH trace before emitting the event, so
  // the click + any API calls + span activity it causes share a single id,
  // but the next click (typically >>10s later) gets its own id instead of
  // joining onto this one.
  document.addEventListener("click", (e) => {
    const target = e.target as HTMLElement | null;
    if (!target) return;
    let el: HTMLElement | null = target;
    for (let depth = 0; el && el !== document.body && depth < 6; depth++, el = el.parentElement) {
      const tag = el.tagName.toLowerCase();
      const role = el.getAttribute("role");
      if (tag === "button" || tag === "a" || role === "button" || role === "tab" || role === "menuitem") {
        const label = (el.getAttribute("aria-label") || el.getAttribute("title") || el.textContent || "").trim().replace(/\s+/g, " ").slice(0, 80);
        beginTrace(); // new id per click — no stale join from a prior action
        trace.event("ui_click", {
          tag,
          label,
          role: role ?? undefined,
          href: tag === "a" ? el.getAttribute("href") ?? undefined : undefined,
        });
        return;
      }
    }
  }, { capture: true, passive: true });

  // Error tracking
  window.addEventListener("error", (e) => {
    trace.event("js_error", {
      message: e.message,
      filename: e.filename?.split("/").pop(),
      lineno: e.lineno,
      colno: e.colno,
    });
  });

  window.addEventListener("unhandledrejection", (e) => {
    trace.event("unhandled_rejection", {
      reason: String(e.reason).slice(0, 200),
    });
  });
}
