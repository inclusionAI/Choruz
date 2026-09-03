import type { ChoruzTransport, DashboardSocket } from "../api/transport";
import { base64UrlToBytes, bytesToBase64Url } from "./remote-control-crypto";
import type { RelayEnvelope, RelayLink } from "./relay-session";

// ---------------------------------------------------------------------------
// A `ChoruzTransport` over a Remote Control session. Every same-origin fetch
// becomes `http.request` (+ chunked `http.body`) frames the host bridge
// executes locally and answers with `http.response` (+ `http.body`); every
// gateway socket becomes a `stream.*` multiplexed over the one transport
// socket. The frame contract is owned by
// services/choruz-api-gateway/src/remote_control_executor.rs.
// ---------------------------------------------------------------------------

/** Raw bytes per frame; mirrors the executor's `CHUNK_BYTES`. */
export const CHUNK_BYTES = 384 * 1024;
const REQUEST_TIMEOUT_MS = 30_000;
const NULL_BODY_STATUSES = new Set([101, 204, 205, 304]);

export type RelayTransport = ChoruzTransport & {
  /** Fails every in-flight request, closes every socket and stops listening. */
  dispose(): void;
};

export type RelayTransportOptions = {
  requestTimeoutMs?: number;
};

type PendingRequest = {
  resolve: (response: Response) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
  head: { status: number; headers: Record<string, string>; chunks: number } | null;
  body: Uint8Array[];
  received: number;
};

export function createRelayTransport(
  link: RelayLink,
  options: RelayTransportOptions = {},
): RelayTransport {
  const timeoutMs = options.requestTimeoutMs ?? REQUEST_TIMEOUT_MS;
  const pending = new Map<string, PendingRequest>();
  const sockets = new Map<string, RelaySocket>();

  const settle = (requestId: string): PendingRequest | undefined => {
    const request = pending.get(requestId);
    if (!request) return undefined;
    pending.delete(requestId);
    clearTimeout(request.timer);
    return request;
  };

  const complete = (requestId: string, request: PendingRequest) => {
    if (!settle(requestId) || !request.head) return;
    const total = request.body.reduce((sum, chunk) => sum + chunk.byteLength, 0);
    const bytes = new Uint8Array(total);
    let offset = 0;
    for (const chunk of request.body) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
    request.resolve(new Response(
      NULL_BODY_STATUSES.has(request.head.status) ? null : bytes,
      { status: request.head.status, headers: request.head.headers },
    ));
  };

  const failAll = (reason: string) => {
    for (const [requestId, request] of pending) {
      settle(requestId);
      request.reject(new TypeError(reason));
    }
    for (const socket of [...sockets.values()]) socket.fail(1006, reason);
  };

  const dispatch = (envelope: RelayEnvelope) => {
    const payload = envelope.payload ?? {};
    switch (envelope.kind) {
      case "http.response": {
        const requestId = String(payload.request_id ?? "");
        const request = pending.get(requestId);
        if (!request) return;
        if (typeof payload.error === "string") {
          settle(requestId);
          request.reject(new TypeError(payload.error));
          return;
        }
        const status = Number(payload.status);
        const chunks = Number(payload.body_chunks ?? 0);
        if (!Number.isSafeInteger(status) || !Number.isSafeInteger(chunks)) {
          settle(requestId);
          request.reject(new TypeError("The paired computer sent an invalid response."));
          return;
        }
        const headers: Record<string, string> = {};
        if (payload.headers && typeof payload.headers === "object") {
          for (const [name, value] of Object.entries(payload.headers as Record<string, unknown>)) {
            if (typeof value === "string") headers[name] = value;
          }
        }
        request.head = { status, headers, chunks };
        request.body = new Array<Uint8Array>(chunks);
        if (chunks === 0) complete(requestId, request);
        return;
      }
      case "http.body": {
        const requestId = String(payload.request_id ?? "");
        const request = pending.get(requestId);
        if (!request?.head) return;
        const index = Number(payload.index);
        if (!Number.isSafeInteger(index) || index < 0 || index >= request.head.chunks) return;
        if (request.body[index] === undefined) request.received += 1;
        request.body[index] = new Uint8Array(base64UrlToBytes(String(payload.data ?? "")));
        if (request.received === request.head.chunks) complete(requestId, request);
        return;
      }
      case "stream.opened":
      case "stream.data":
      case "stream.close":
        sockets.get(String(payload.stream_id ?? ""))?.handleFrame(envelope.kind, payload);
        return;
      default:
    }
  };

  const unsubscribe = link.subscribe(dispatch);
  const unsubscribeStatus = link.onStatus((status) => {
    if (status !== "connected") failAll("The paired computer is not connected.");
  });

  return {
    async fetch(input, init = {}) {
      const path = relayPath(input);
      const method = (init.method ?? "GET").toUpperCase();
      const headers = headersToObject(init.headers);
      const body = await bodyBytes(init.body, headers);
      const chunks = chunk(body);
      const requestId = crypto.randomUUID();
      const response = new Promise<Response>((resolve, reject) => {
        pending.set(requestId, {
          resolve,
          reject,
          timer: setTimeout(() => {
            settle(requestId);
            reject(new TypeError("The paired computer did not respond in time."));
          }, timeoutMs),
          head: null,
          body: [],
          received: 0,
        });
      });
      try {
        await link.send({
          kind: "http.request",
          payload: { request_id: requestId, method, path, headers, body_chunks: chunks.length },
        });
        for (const [index, data] of chunks.entries()) {
          await link.send({
            kind: "http.body",
            payload: { request_id: requestId, index, data: bytesToBase64Url(data) },
          });
        }
      } catch (error) {
        const request = settle(requestId);
        request?.reject(new TypeError(error instanceof Error ? error.message : String(error)));
      }
      return response;
    },
    socket(path) {
      const streamId = crypto.randomUUID();
      const socket = new RelaySocket(link, streamId, path, () => sockets.delete(streamId));
      sockets.set(streamId, socket);
      return socket;
    },
    dispose() {
      unsubscribe();
      unsubscribeStatus();
      failAll("The remote dashboard was closed.");
    },
  };
}

/** The same-origin path of a dashboard call; absolute URLs keep only their
 *  path and query because the host resolves the origin. */
export function relayPath(input: string): string {
  if (input.startsWith("/")) return input;
  const url = new URL(input, "http://dashboard.invalid");
  return `${url.pathname}${url.search}`;
}

function headersToObject(init: HeadersInit | undefined): Record<string, string> {
  const headers: Record<string, string> = {};
  if (!init) return headers;
  new Headers(init).forEach((value, name) => {
    headers[name] = value;
  });
  return headers;
}

async function bodyBytes(
  body: BodyInit | null | undefined,
  headers: Record<string, string>,
): Promise<Uint8Array> {
  if (body === null || body === undefined) return new Uint8Array();
  const setContentType = (value: string) => {
    if (!("content-type" in headers)) headers["content-type"] = value;
  };
  if (typeof body === "string") {
    setContentType("text/plain;charset=UTF-8");
    return new TextEncoder().encode(body);
  }
  if (body instanceof URLSearchParams) {
    setContentType("application/x-www-form-urlencoded;charset=UTF-8");
    return new TextEncoder().encode(body.toString());
  }
  if (body instanceof ArrayBuffer) return new Uint8Array(body);
  if (ArrayBuffer.isView(body)) {
    return new Uint8Array(body.buffer, body.byteOffset, body.byteLength);
  }
  if (typeof Blob !== "undefined" && body instanceof Blob) {
    if (body.type) setContentType(body.type);
    return new Uint8Array(await body.arrayBuffer());
  }
  throw new TypeError("This request body cannot be relayed to the paired computer.");
}

function chunk(bytes: Uint8Array): Uint8Array[] {
  const chunks: Uint8Array[] = [];
  for (let offset = 0; offset < bytes.byteLength; offset += CHUNK_BYTES) {
    chunks.push(bytes.subarray(offset, Math.min(offset + CHUNK_BYTES, bytes.byteLength)));
  }
  return chunks;
}

function closeEvent(code: number, reason: string, wasClean: boolean): CloseEvent {
  if (typeof CloseEvent === "function") {
    return new CloseEvent("close", { code, reason, wasClean });
  }
  return Object.assign(new Event("close"), { code, reason, wasClean }) as CloseEvent;
}

const CONNECTING = 0;
const OPEN = 1;
const CLOSING = 2;
const CLOSED = 3;

/** A `DashboardSocket` carried as `stream.*` frames over the relay link. */
export class RelaySocket implements DashboardSocket {
  binaryType: BinaryType = "blob";
  readyState = CONNECTING;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  private inbound: Uint8Array[] = [];

  constructor(
    private readonly link: RelayLink,
    private readonly streamId: string,
    path: string,
    private readonly onClosed: () => void,
  ) {
    this.link
      .send({ kind: "stream.open", payload: { stream_id: streamId, path } })
      .catch((error: unknown) => this.fail(1006, error instanceof Error ? error.message : String(error)));
  }

  send(data: string | ArrayBufferLike | ArrayBufferView): void {
    if (this.readyState !== OPEN) throw new Error("The relayed socket is not open.");
    const binary = typeof data !== "string";
    const bytes = typeof data === "string"
      ? new TextEncoder().encode(data)
      : ArrayBuffer.isView(data)
        ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength)
        : new Uint8Array(data);
    const chunks = bytes.byteLength === 0 ? [new Uint8Array()] : chunk(bytes);
    chunks.forEach((piece, index) => {
      this.link
        .send({
          kind: "stream.data",
          payload: {
            stream_id: this.streamId,
            encoding: binary ? "binary" : "text",
            data: bytesToBase64Url(piece),
            last: index + 1 === chunks.length,
          },
        })
        .catch((error: unknown) => this.fail(1006, error instanceof Error ? error.message : String(error)));
    });
  }

  close(code = 1000, reason = ""): void {
    if (this.readyState >= CLOSING) return;
    this.readyState = CLOSING;
    this.link
      .send({ kind: "stream.close", payload: { stream_id: this.streamId, code, reason } })
      .catch(() => undefined);
    this.finish(code, reason, true);
  }

  handleFrame(kind: string, payload: Record<string, unknown>): void {
    if (this.readyState === CLOSED) return;
    switch (kind) {
      case "stream.opened":
        if (this.readyState !== CONNECTING) return;
        this.readyState = OPEN;
        this.onopen?.(new Event("open"));
        return;
      case "stream.data": {
        this.inbound.push(new Uint8Array(base64UrlToBytes(String(payload.data ?? ""))));
        if (payload.last === false) return;
        const parts = this.inbound;
        this.inbound = [];
        const total = parts.reduce((sum, part) => sum + part.byteLength, 0);
        const bytes = new Uint8Array(total);
        let offset = 0;
        for (const part of parts) {
          bytes.set(part, offset);
          offset += part.byteLength;
        }
        const data = payload.encoding === "binary"
          ? this.binaryType === "arraybuffer" ? bytes.buffer : new Blob([bytes])
          : new TextDecoder().decode(bytes);
        this.onmessage?.(new MessageEvent("message", { data }));
        return;
      }
      case "stream.close": {
        const code = Number.isSafeInteger(payload.code) ? Number(payload.code) : 1006;
        this.finish(code, typeof payload.reason === "string" ? payload.reason : "", code === 1000);
        return;
      }
      default:
    }
  }

  fail(code: number, reason: string): void {
    if (this.readyState === CLOSED) return;
    this.onerror?.(new Event("error"));
    this.finish(code, reason, false);
  }

  private finish(code: number, reason: string, wasClean: boolean): void {
    if (this.readyState === CLOSED) return;
    this.readyState = CLOSED;
    this.onClosed();
    this.onclose?.(closeEvent(code, reason, wasClean));
  }
}
