// ---------------------------------------------------------------------------
// The browser's two ways out: same-origin HTTP under `/api` and long-lived
// gateway sockets. Every client-side call goes through the active transport
// so a remote dashboard can swap the local one for a relay-backed one
// without touching the callers.
// ---------------------------------------------------------------------------

export interface ChoruzTransport {
  /** An HTTP call to a same-origin `/api/...` path (a Next.js route, or the
   *  `/api/v1/*` rewrite to the gateway). */
  fetch(input: string, init?: RequestInit): Promise<Response>;
  /** A long-lived socket to a gateway path such as `/v1/ws/sync?...` or
   *  `/v1/ws/terminals/<binding>?...`. */
  socket(path: string, options?: SocketOptions): DashboardSocket;
}

/**
 * The part of the `WebSocket` surface the dashboard uses. A browser
 * `WebSocket` satisfies it; the relay transport implements it over frames.
 */
export interface DashboardSocket {
  binaryType: BinaryType;
  readonly readyState: number;
  onopen: ((event: Event) => void) | null;
  onmessage: ((event: MessageEvent) => void) | null;
  onclose: ((event: CloseEvent) => void) | null;
  onerror: ((event: Event) => void) | null;
  send(data: string | ArrayBufferLike | ArrayBufferView): void;
  close(code?: number, reason?: string): void;
}

export type SocketOptions = {
  /** Gateway origin to use instead of the page's host and the configured
   *  API port, e.g. when the page is served by a remote Choruz. */
  gatewayBaseUrl?: string;
};

/**
 * The `ws(s)://` URL of a gateway socket path. Next.js rewrites only proxy
 * HTTP, so sockets go straight to the gateway: the given origin, else the
 * page's hostname on `NEXT_PUBLIC_CHORUZ_API_PORT` (works for localhost and
 * LAN access alike).
 */
export function gatewaySocketUrl(path: string, gatewayBaseUrl?: string): string {
  if (gatewayBaseUrl) {
    return `${gatewayBaseUrl.replace(/^http/, "ws")}${path}`;
  }
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const port = process.env.NEXT_PUBLIC_CHORUZ_API_PORT?.trim() || "3000";
  return `${protocol}//${window.location.hostname}:${port}${path}`;
}

/** The dashboard served by the Choruz it talks to. */
export const localTransport: ChoruzTransport = {
  fetch: (input, init) => globalThis.fetch(input, init),
  socket: (path, options) => new WebSocket(gatewaySocketUrl(path, options?.gatewayBaseUrl)),
};

let active: ChoruzTransport = localTransport;

/** Install the transport every client call uses; `null` restores the local one. */
export function setActiveTransport(transport: ChoruzTransport | null): void {
  active = transport ?? localTransport;
}

export function activeTransport(): ChoruzTransport {
  return active;
}

/** `fetch` through the active transport. On the server there is no
 *  transport to swap, so the call goes straight to `fetch`. */
export function transportFetch(input: string, init?: RequestInit): Promise<Response> {
  if (typeof window === "undefined") return globalThis.fetch(input, init);
  return active.fetch(input, init);
}

export function transportSocket(path: string, options?: SocketOptions): DashboardSocket {
  return active.socket(path, options);
}
