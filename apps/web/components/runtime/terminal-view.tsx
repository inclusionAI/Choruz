"use client";

import { useEffect, useRef, useCallback } from "react";
import { useTheme } from "next-themes";
import { trace } from "../../lib/api/choruz-trace";
import { type DashboardSocket, transportSocket } from "../../lib/api/transport";
import { createTerminalWriteBuffer } from "../../lib/terminal/terminal-write-buffer";

type TerminalViewProps = {
  bindingId: string;
  sessionToken: string;
  gatewayBaseUrl?: string;
};

const MAX_RETRIES = 30;
const RETRY_DELAY = 3000;

// xterm.js owns its own canvas — CSS variables don't reach inside, so the
// terminal palette has to be a JS object pushed in at construction and
// updated on theme switch via `term.options.theme = ...`.
//
// DARK_THEME is the original Windows-Terminal-ish high-contrast palette.
// LIGHT_THEME is Solarized-Light-ish, tuned to sit on the Anthropic cream
// (#FAF9F5) the rest of the app uses — terminal output reads with the
// same warmth as the chat surface around it instead of being a black
// rectangle dropped into a cream page.

const DARK_THEME = {
  background: "#0c0c0c",
  foreground: "#d4d4d4",
  cursor: "#d4d4d4",
  selectionBackground: "rgba(255,255,255,0.2)",
  black: "#000000",
  red: "#e74856",
  green: "#16c60c",
  yellow: "#f9f1a5",
  blue: "#3b78ff",
  magenta: "#b4009e",
  cyan: "#61d6d6",
  white: "#cccccc",
  brightBlack: "#767676",
  brightRed: "#e74856",
  brightGreen: "#16c60c",
  brightYellow: "#f9f1a5",
  brightBlue: "#3b78ff",
  brightMagenta: "#b4009e",
  brightCyan: "#61d6d6",
  brightWhite: "#ffffff",
} as const;

const LIGHT_THEME = {
  background: "#FAF9F5",
  foreground: "#141413",
  cursor: "#141413",
  selectionBackground: "rgba(20,20,19,0.18)",
  // Solarized-Light ANSI for readable mid-saturation on cream
  black: "#073642",
  red: "#cb4b16",
  green: "#859900",
  yellow: "#b58900",
  blue: "#268bd2",
  magenta: "#d33682",
  cyan: "#2aa198",
  white: "#586e75",
  brightBlack: "#586e75",
  brightRed: "#dc322f",
  brightGreen: "#859900",
  brightYellow: "#b58900",
  brightBlue: "#268bd2",
  brightMagenta: "#6c71c4",
  brightCyan: "#2aa198",
  brightWhite: "#073642",
} as const;

export function TerminalView({ bindingId, sessionToken, gatewayBaseUrl }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<import("@xterm/xterm").Terminal | null>(null);
  const wsRef = useRef<DashboardSocket | null>(null);
  const fitRef = useRef<import("@xterm/addon-fit").FitAddon | null>(null);
  const mountedRef = useRef(false);
  const retriesRef = useRef(0);
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const writeBufferRef = useRef<ReturnType<typeof createTerminalWriteBuffer> | null>(null);
  const { resolvedTheme } = useTheme();

  const connectWs = useCallback(
    (term: import("@xterm/xterm").Terminal, signal: AbortSignal) => {
      if (signal.aborted || !mountedRef.current) return;

      const socketPath = `/v1/ws/terminals/${encodeURIComponent(bindingId)}?token=${encodeURIComponent(sessionToken)}&cols=${term.cols}&rows=${term.rows}`;
      const wsSpan = trace.start("terminal_ws_connect", { bindingId, path: socketPath.replace(/token=[^&]+/, "token=***") });
      // Ends on the first frame after open: the CLI's own startup time as the
      // user experiences it, separate from the handshake above.
      let firstOutputSpan: ReturnType<typeof trace.start> | null = null;
      const ws = transportSocket(socketPath, { gatewayBaseUrl });
      ws.binaryType = "arraybuffer";
      wsRef.current = ws;
      writeBufferRef.current?.dispose();
      const writeBuffer = createTerminalWriteBuffer((data) => term.write(data));
      writeBufferRef.current = writeBuffer;

      ws.onopen = () => {
        if (signal.aborted) {
          ws.close();
          return;
        }
        wsSpan.end({ status: "connected", retry: retriesRef.current });
        if (retriesRef.current > 0) {
          term.write(`\r\n\x1b[32mTerminal reconnected, restoring session...\x1b[0m\r\n`);
        }
        retriesRef.current = 0;
        term.clear();
        term.focus();
        firstOutputSpan = trace.start("terminal_first_output", { bindingId });
      };

      ws.onmessage = (event: MessageEvent) => {
        if (firstOutputSpan) {
          const bytes = event.data instanceof ArrayBuffer ? event.data.byteLength : String(event.data).length;
          firstOutputSpan.end({ bytes });
          firstOutputSpan = null;
        }
        if (event.data instanceof ArrayBuffer) {
          writeBuffer.enqueue(new Uint8Array(event.data));
        } else if (typeof event.data === "string") {
          writeBuffer.enqueue(event.data);
        }
      };

      ws.onclose = (e) => {
        trace.event("terminal_ws_close", { bindingId, code: e.code, reason: e.reason, retry: retriesRef.current });
        if (signal.aborted || !mountedRef.current) return;
        if (retriesRef.current < MAX_RETRIES) {
          retriesRef.current += 1;
          term.write(`\r\n\x1b[33mReconnecting terminal (will auto-restore session)... (${retriesRef.current}/${MAX_RETRIES})\x1b[0m\r\n`);
          retryTimerRef.current = setTimeout(() => {
            if (!signal.aborted) connectWs(term, signal);
          }, RETRY_DELAY);
        } else {
          term.write("\r\n\x1b[31mFailed to connect after multiple retries.\x1b[0m\r\n");
        }
      };

      ws.onerror = () => {
        wsSpan.end({ status: "error" });
        // onclose will fire after onerror — retry logic is there
      };

      // User input → WebSocket
      term.onData((data: string) => {
        if (ws.readyState === WebSocket.OPEN) {
          try { ws.send(data); } catch (err) { trace.event("terminal_send_error", { bindingId, error: String(err) }); }
        } else {
          trace.event("terminal_send_dropped", { bindingId, readyState: ws.readyState });
        }
      });

      term.onBinary((data: string) => {
        if (ws.readyState === WebSocket.OPEN) {
          try {
            const bytes = new Uint8Array(data.length);
            for (let i = 0; i < data.length; i++) bytes[i] = data.charCodeAt(i);
            ws.send(bytes);
          } catch (err) { trace.event("terminal_send_error", { bindingId, binary: true, error: String(err) }); }
        }
      });
    },
    [bindingId, sessionToken, gatewayBaseUrl],
  );

  const connect = useCallback(
    async (container: HTMLDivElement, signal: AbortSignal) => {
      const mountSpan = trace.start("terminal_mount", { bindingId });
      const [{ Terminal }, { FitAddon }] = await Promise.all([
        import("@xterm/xterm"),
        import("@xterm/addon-fit"),
        import("@xterm/xterm/css/xterm.css"),
      ]);

      // React Strict Mode intentionally mounts, cleans up, and mounts again in
      // development. The dynamic imports above can outlive the first mount;
      // without a mount-local cancellation signal both continuations append an
      // xterm instance and open a WebSocket in the same container.
      if (signal.aborted) {
        mountSpan.end({ status: "cancelled" });
        return;
      }

      const fitAddon = new FitAddon();
      // Pick the palette that matches the active app theme. We read
      // resolvedTheme on each connect (mount); subsequent switches go
      // through the live-update effect below without rebuilding the
      // terminal.
      const initialPalette =
        document.documentElement.getAttribute("data-theme") === "dark"
          ? DARK_THEME
          : LIGHT_THEME;
      const term = new Terminal({
        cursorBlink: true,
        fontSize: 14,
        fontFamily:
          '"SF Mono", "Fira Code", "Cascadia Code", "JetBrains Mono", Menlo, monospace',
        theme: { ...initialPalette },
        allowProposedApi: true,
      });

      term.loadAddon(fitAddon);
      term.open(container);
      // Delay fit so the browser lays out the container first
      setTimeout(() => {
        if (!signal.aborted) fitAddon.fit();
      }, 50);

      termRef.current = term;
      fitRef.current = fitAddon;

      term.write("\x1b[33mConnecting to terminal...\x1b[0m\r\n");
      mountSpan.end({ cols: term.cols, rows: term.rows });
      connectWs(term, signal);
    },
    [connectWs, bindingId],
  );

  useEffect(() => {
    if (!containerRef.current || mountedRef.current) return;
    const lifecycle = new AbortController();
    mountedRef.current = true;

    connect(containerRef.current, lifecycle.signal).catch((err) => {
      if (lifecycle.signal.aborted) return;
      trace.event("terminal_mount_error", { bindingId, error: String(err) });
    });

    const handleResize = () => {
      const fit = fitRef.current;
      const term = termRef.current;
      const ws = wsRef.current;
      if (!fit || !term) return;
      // fitAddon.fit() reads `renderer.dimensions`; on the very first
      // ResizeObserver tick after `display:none → flex` (tab switch),
      // the renderer can briefly be undefined and crash with
      // "Cannot read properties of undefined (reading 'dimensions')".
      // Swallow that one race — the next ResizeObserver tick (fired
      // synchronously when the renderer finishes attaching) re-fits
      // correctly, so the terminal still ends up at the right size.
      try {
        fit.fit();
      } catch (err) {
        trace.event("terminal_fit_skipped", { bindingId, error: String(err) });
        return;
      }
      if (ws && ws.readyState === WebSocket.OPEN) {
        try {
          ws.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
        } catch (err) { trace.event("terminal_resize_send_error", { bindingId, error: String(err) }); }
      }
    };

    const resizeObserver = new ResizeObserver(handleResize);
    if (containerRef.current) {
      resizeObserver.observe(containerRef.current);
    }

    return () => {
      lifecycle.abort();
      mountedRef.current = false;
      if (retryTimerRef.current) clearTimeout(retryTimerRef.current);
      resizeObserver.disconnect();
      wsRef.current?.close();
      writeBufferRef.current?.dispose();
      termRef.current?.dispose();
    };
  }, [connect]);

  // Live theme switch — mutate the xterm options directly so the
  // existing PTY session keeps its scrollback. Rebuilding the terminal
  // would reconnect the WebSocket and erase the buffer.
  // Wrapped in try/catch because setting .options.theme triggers an
  // internal re-render that, when the terminal is in a transient
  // attach state (e.g. just after `display:none → flex`), can throw
  // the same "renderer.dimensions undefined" race the resize path
  // hits. Swallowing keeps the colours stale for one frame; the next
  // resize tick repaints in the new palette.
  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    const next = resolvedTheme === "dark" ? DARK_THEME : LIGHT_THEME;
    try {
      term.options.theme = { ...next };
    } catch (err) {
      trace.event("terminal_theme_apply_skipped", { bindingId, error: String(err) });
    }
  }, [resolvedTheme, bindingId]);

  return <div ref={containerRef} className="terminal-container" />;
}
