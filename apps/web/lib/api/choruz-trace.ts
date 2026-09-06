import { sanitizeTelemetryData } from "./telemetry-sanitize";
import { activeTransport, setRequestObserver } from "./transport";
import { ActivityOutbox, type ActivityEvent } from "./activity-outbox";

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
let _scope: string | null = null;
let _sessionId = "";
let _outbox: ActivityOutbox | null = null;
let _delivery: ReturnType<typeof activeTransport> | null = null;
let _context: Record<string, string | null> = {};

/** Set context for new events; already-started spans retain their captured context. */
export function setActivityContext(context: {
  company_id: string | null;
  conversation_id: string | null;
  binding_id?: string | null;
  runtime_host_id?: string | null;
  harness_account_id?: string | null;
}) {
  _context = context;
}

/** Call once at app startup to wire up the telemetry endpoint. */
export function initTrace(sessionToken: string, gatewayBase: string, principalId: string) {
  _sessionToken = sessionToken;
  _scope = `${gatewayBase}\0${principalId}`;
  _delivery = activeTransport();
  _sessionId = crypto.randomUUID();
  setRequestObserver((input, init) => {
    const resource = new URL(input, window.location.origin).pathname;
    const span = trace.start("http_request", { resource, method: init?.method ?? "GET" });
    return { traceId: span.traceId, finish: result => span.end(result) };
  });
  _outbox ??= new ActivityOutbox();
  scheduleFlush();
  const session = _sessionId;
  return () => {
    if (_sessionId !== session) return;
    _scope = null;
    _sessionToken = null;
    _delivery = null;
    _context = {};
    setRequestObserver(null);
    if (_flushTimer) clearTimeout(_flushTimer);
    _flushTimer = null;
    endTrace();
  };
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
  return crypto.randomUUID().replaceAll("-", "");
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

export interface TraceEntry extends ActivityEvent {
  source: "FE";
}

function spanId(): string {
  return crypto.randomUUID();
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

let _flushTimer: ReturnType<typeof setTimeout> | null = null;
const FLUSH_INTERVAL = 2000; // batch every 2s
let retryDelay = FLUSH_INTERVAL;

function enqueue(entry: TraceEntry, scope = _scope) {
  if (!_outbox || !scope) return;
  const { source: _source, ...event } = entry;
  void _outbox.append(scope, event).catch(() => {
    console.warn("[choruz-trace] activity queue unavailable; event not persisted");
  });
  scheduleFlush();
}

function scheduleFlush() {
  if (!_flushTimer) {
    _flushTimer = setTimeout(flush, retryDelay);
  }
}

async function flush() {
  _flushTimer = null;
  if (!_outbox || !_scope || !_sessionToken || !_delivery) return;
  try {
    await _outbox.flush(_scope, _sessionToken, _delivery);
    retryDelay = FLUSH_INTERVAL;
  } catch {
    retryDelay = Math.min(retryDelay * 2, 60_000);
    console.warn("[choruz-trace] activity delivery pending; retrying retained events");
  } finally {
    if (_scope) scheduleFlush();
  }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export const trace = {
  /** Start a timed span. Call span.end() when done. */
  start(name: string, data?: Record<string, unknown>): Span {
    data = { ..._context, ...data };
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
    const scope = _scope;

    const entry: TraceEntry = { eventId: crypto.randomUUID(), schemaVersion: 1, sessionId: _sessionId, source: "FE", traceId: tid, spanId: sid, name, ts, data: sanitizeTelemetryData(data) };
    let ended = false;

    pushRing({ ...entry, data: { ...entry.data, phase: "started" } });
    enqueue({ ...entry, data: { ...entry.data, phase: "started" } }, scope);

    // Log start immediately
    console.debug(`[choruz] ▶ ${name}`, sanitizeTelemetryData(data) ?? "", `trace=${tid}`);

    return {
      traceId: tid,
      end(extra?: Record<string, unknown>) {
        if (ended) return;
        ended = true;
        const durationMs = Math.round(performance.now() - t0);
        const merged = sanitizeTelemetryData({ ...data, ...extra }) ?? {};
        const final_entry: TraceEntry = {
          ...entry,
          eventId: crypto.randomUUID(),
          ts: new Date().toISOString(),
          durationMs,
          data: { ...merged, phase: "finished", outcome: merged.outcome ?? (extra?.error ? "failed" : "succeeded") },
        };
        pushRing(final_entry);
        enqueue(final_entry, scope);

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
    data = { ..._context, ...data };
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
      eventId: crypto.randomUUID(),
      schemaVersion: 1,
      sessionId: _sessionId,
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
// Authenticated UI activity; never record input values or raw keystrokes.
// ---------------------------------------------------------------------------

let _interactionListenersAttached = false;

/** Start bounded UI observation after initTrace; return cleanup for the owning mount. */
export function startInteractionTracking() {
  if (_interactionListenersAttached || typeof window === "undefined") return;
  _interactionListenersAttached = true;
  const controller = new AbortController();
  const options = { capture: true, passive: true, signal: controller.signal };
  let scrollTimer: ReturnType<typeof setTimeout> | undefined;
  const describe = (el: Element) => ({
    tag: el.tagName.toLowerCase(),
    control: (el.getAttribute("data-activity") || el.getAttribute("data-testid") || el.id || el.getAttribute("class") || el.tagName.toLowerCase()).slice(0, 160),
    label: el.closest('.attachment-queue,.msg-group,.conv-item,.folder-picker-modal,.file-tree') ? "" : (el.getAttribute("aria-label") || el.getAttribute("title") || el.getAttribute("name") || "").slice(0, 80),
    role: el.getAttribute("role") ?? undefined,
  });

  // Click tracking — walk up to nearest actionable element and report its
  // label. Every click starts a FRESH trace before emitting the event, so
  // the click + any API calls + span activity it causes share a single id,
  // but the next click (typically >>10s later) gets its own id instead of
  // joining onto this one.
  document.addEventListener("click", (e) => {
    if (!(e.target instanceof Element)) return;
    const el = e.target.closest('button,a,[role="button"],[role="tab"],[role="menuitem"],[role="option"],input,select');
    if (!el || el.closest('[data-activity-private]')) return;
    beginTrace();
    trace.event("ui_click", describe(el));
  }, options);

  document.addEventListener("change", e => {
    const el = e.target;
    if (!(el instanceof Element) || el.closest('[data-activity-private]')) return;
    if (el instanceof HTMLSelectElement) {
      trace.event("ui_selection", { ...describe(el), selected_index: el.selectedIndex });
    } else if (el instanceof HTMLInputElement && ["checkbox", "radio"].includes(el.type)) {
      trace.event("ui_toggle", { ...describe(el), checked: el.checked });
    } else if (el instanceof HTMLInputElement && el.type === "file") {
      trace.event("ui_files_selected", { ...describe(el), count: el.files?.length ?? 0 });
    }
  }, options);

  document.addEventListener("submit", e => {
    if (e.target instanceof Element && !e.target.closest('[data-activity-private]')) trace.event("ui_submit", describe(e.target));
  }, options);
  document.addEventListener("keydown", e => {
    if (!["Escape", "Enter"].includes(e.key) && !((e.metaKey || e.ctrlKey) && ["s", "k", "f"].includes(e.key.toLowerCase()))) return;
    if (!(e.target instanceof Element) || e.target.closest('[data-activity-private],.terminal-view,input[type="password"]')) return;
    beginTrace();
    trace.event("ui_command", { ...describe(e.target), command: e.key, modifier: e.metaKey || e.ctrlKey });
  }, options);
  document.addEventListener("visibilitychange", () => {
    trace.event("page_visibility", { state: document.visibilityState });
  }, options);
  for (const action of ["copy", "paste"] as const) {
    document.addEventListener(action, e => {
      if (e.target instanceof Element && !e.target.closest('[data-activity-private],input[type="password"]')) {
        trace.event(`ui_${action}`, describe(e.target));
      }
    }, options);
  }
  document.addEventListener("scroll", e => {
    const el = e.target;
    if (!(el instanceof Element) || el.closest('[data-activity-private],.terminal-view')) return;
    clearTimeout(scrollTimer);
    scrollTimer = setTimeout(() => {
      const extent = el.scrollHeight - el.clientHeight;
      trace.event("ui_scroll", { ...describe(el), percent: extent > 0 ? Math.round(el.scrollTop / extent * 100) : 0 });
    }, 300);
  }, options);
  window.addEventListener("pagehide", () => trace.event("page_left"), options);
  trace.event("page_opened", { resource: window.location.pathname });

  // Error tracking
  window.addEventListener("error", (e) => {
    trace.event("js_error", {
      message: e.message,
      filename: e.filename?.split("/").pop(),
      lineno: e.lineno,
      colno: e.colno,
    });
  }, options);

  window.addEventListener("unhandledrejection", (e) => {
    trace.event("unhandled_rejection", {
      reason: String(e.reason).slice(0, 200),
    });
  }, options);
  return () => {
    controller.abort();
    clearTimeout(scrollTimer);
    _interactionListenersAttached = false;
  };
}
