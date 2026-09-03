import { decryptWithSessionKey, encryptWithSessionKey } from "./remote-control-crypto";

// ---------------------------------------------------------------------------
// The paired browser's end of the Remote Control transport: the rendezvous
// socket that waits for the host's transport offer, the transport socket the
// offer names, and the AES-GCM envelope every frame travels in. Nothing here
// knows what the frames mean; `relay-transport.ts` does.
// ---------------------------------------------------------------------------

/** What `pair.complete` hands a browser and what it keeps to reconnect. */
export type RemoteCredentials = {
  device_id: string;
  gateway_url: string;
  gateway_ticket: string;
  session_key: string;
};

/** A decrypted frame. `sync_cursor` marks a projected sync change the host
 *  expects an acknowledgement for. */
export type RelayEnvelope = {
  kind: string;
  payload?: Record<string, unknown>;
  sync_cursor?: number;
};

export type RelayStatus =
  /** The rendezvous socket is opening or reconnecting. */
  | "connecting"
  /** The rendezvous socket is open and no transport is connected. */
  | "waiting"
  /** The transport socket is open and the host has our `device.hello`. */
  | "connected"
  /** The Cloud Gateway closed the session with 4003: the device was revoked. */
  | "revoked"
  /** `close()` was called. */
  | "closed";

/** The narrow surface the relay transport needs from a session, so tests can
 *  drive it without sockets. */
export interface RelayLink {
  readonly status: RelayStatus;
  /** Encrypts and sends one envelope. Envelopes leave in call order. Rejects
   *  when no transport is connected. */
  send(envelope: RelayEnvelope): Promise<void>;
  subscribe(handler: (envelope: RelayEnvelope) => void): () => void;
  onStatus(handler: (status: RelayStatus) => void): () => void;
  close(): void;
}

export type SocketFactory = (url: string) => WebSocket;

export const DEVICE_REVOKED_CLOSE_CODE = 4003;
const RECONNECT_DELAY_MS = 2000;
const SOCKET_OPEN = 1;

/** The Cloud Gateway's `/connect` URL for a ticket or opaque pairing id. */
export function gatewayConnectUrl(gatewayUrl: string, params: Record<string, string>): string {
  const url = new URL(gatewayUrl);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.pathname = "/connect";
  url.search = new URLSearchParams(params).toString();
  return url.toString();
}

export type RelaySessionOptions = {
  createSocket?: SocketFactory;
  reconnectDelayMs?: number;
};

export class RelaySession implements RelayLink {
  private readonly credentials: RemoteCredentials;
  private readonly createSocket: SocketFactory;
  private readonly reconnectDelayMs: number;
  private rendezvous: WebSocket | null = null;
  private transport: WebSocket | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly handlers = new Set<(envelope: RelayEnvelope) => void>();
  private readonly statusHandlers = new Set<(status: RelayStatus) => void>();
  private currentStatus: RelayStatus = "closed";
  private stopped = false;
  // Encryption and decryption are asynchronous; chaining keeps frames in the
  // order they were sent and received, which chunked bodies rely on.
  private sendChain: Promise<void> = Promise.resolve();
  private receiveChain: Promise<void> = Promise.resolve();

  constructor(credentials: RemoteCredentials, options: RelaySessionOptions = {}) {
    this.credentials = credentials;
    this.createSocket = options.createSocket ?? ((url) => new WebSocket(url));
    this.reconnectDelayMs = options.reconnectDelayMs ?? RECONNECT_DELAY_MS;
  }

  get status(): RelayStatus {
    return this.currentStatus;
  }

  start(): void {
    if (this.stopped || this.rendezvous) return;
    this.connectRendezvous();
  }

  close(): void {
    this.stopped = true;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    const { transport, rendezvous } = this;
    this.transport = null;
    this.rendezvous = null;
    transport?.close(1000, "Session closed");
    rendezvous?.close(1000, "Session closed");
    this.setStatus("closed");
  }

  subscribe(handler: (envelope: RelayEnvelope) => void): () => void {
    this.handlers.add(handler);
    return () => this.handlers.delete(handler);
  }

  onStatus(handler: (status: RelayStatus) => void): () => void {
    this.statusHandlers.add(handler);
    return () => this.statusHandlers.delete(handler);
  }

  send(envelope: RelayEnvelope): Promise<void> {
    const socket = this.transport;
    if (!socket || socket.readyState !== SOCKET_OPEN) {
      return Promise.reject(new Error("The paired computer is not connected."));
    }
    const next = this.sendChain.then(async () => {
      const encrypted = await encryptWithSessionKey(this.credentials.session_key, envelope);
      if (this.transport !== socket || socket.readyState !== SOCKET_OPEN) {
        throw new Error("The paired computer is not connected.");
      }
      socket.send(JSON.stringify({ kind: "e2e", ...encrypted }));
    });
    this.sendChain = next.catch(() => undefined);
    return next;
  }

  private setStatus(status: RelayStatus): void {
    if (this.currentStatus === status) return;
    this.currentStatus = status;
    for (const handler of this.statusHandlers) handler(status);
  }

  private connectRendezvous(): void {
    this.setStatus("connecting");
    const socket = this.createSocket(gatewayConnectUrl(this.credentials.gateway_url, {
      ticket: this.credentials.gateway_ticket,
    }));
    this.rendezvous = socket;
    socket.onopen = () => {
      if (this.rendezvous !== socket) return;
      this.setStatus(this.transport?.readyState === SOCKET_OPEN ? "connected" : "waiting");
    };
    socket.onmessage = (event) => {
      let value: { kind?: unknown; payload?: unknown };
      try {
        value = JSON.parse(String(event.data)) as { kind?: unknown; payload?: unknown };
      } catch {
        return;
      }
      if (value.kind === "session.offer" && value.payload && typeof value.payload === "object") {
        this.connectTransport(value.payload as Record<string, unknown>);
      }
    };
    socket.onerror = () => undefined;
    socket.onclose = (event) => {
      if (this.rendezvous !== socket) return;
      this.rendezvous = null;
      if (event.code === DEVICE_REVOKED_CLOSE_CODE) {
        this.stopped = true;
        const transport = this.transport;
        this.transport = null;
        transport?.close(1000, "Device revoked");
        this.setStatus("revoked");
        return;
      }
      if (this.stopped) return;
      this.setStatus("connecting");
      this.reconnectTimer = setTimeout(() => {
        this.reconnectTimer = null;
        if (!this.stopped) this.connectRendezvous();
      }, this.reconnectDelayMs);
    };
  }

  private connectTransport(offer: Record<string, unknown>): void {
    const { gateway_url: gatewayUrl, gateway_ticket: ticket } = offer;
    if (typeof gatewayUrl !== "string" || typeof ticket !== "string") return;
    const previous = this.transport;
    this.transport = null;
    previous?.close(1000, "Transport rotated");
    const socket = this.createSocket(gatewayConnectUrl(gatewayUrl, { ticket }));
    this.transport = socket;
    this.setStatus(this.rendezvous?.readyState === SOCKET_OPEN ? "waiting" : "connecting");
    socket.onopen = () => {
      if (this.transport !== socket) return;
      this.send({ kind: "device.hello", payload: { device_id: this.credentials.device_id } })
        .then(() => {
          if (this.transport === socket) this.setStatus("connected");
        })
        .catch(() => socket.close(1011, "Could not greet the host"));
    };
    socket.onmessage = (event) => {
      const raw = String(event.data);
      this.receiveChain = this.receiveChain
        .then(() => this.receive(socket, raw))
        .catch(() => undefined);
    };
    socket.onerror = () => undefined;
    socket.onclose = () => {
      if (this.transport !== socket) return;
      this.transport = null;
      if (this.stopped) return;
      this.setStatus(this.rendezvous?.readyState === SOCKET_OPEN ? "waiting" : "connecting");
    };
  }

  private async receive(socket: WebSocket, raw: string): Promise<void> {
    let value: { kind?: unknown; iv?: unknown; ciphertext?: unknown };
    try {
      value = JSON.parse(raw) as { kind?: unknown; iv?: unknown; ciphertext?: unknown };
    } catch {
      return;
    }
    if (value.kind !== "e2e" || typeof value.iv !== "string" || typeof value.ciphertext !== "string") {
      return;
    }
    let envelope: RelayEnvelope;
    try {
      envelope = await decryptWithSessionKey<RelayEnvelope>(
        this.credentials.session_key, value.iv, value.ciphertext,
      );
    } catch {
      return;
    }
    if (this.transport !== socket || !envelope || typeof envelope.kind !== "string") return;
    for (const handler of this.handlers) handler(envelope);
    if (Number.isSafeInteger(envelope.sync_cursor)) {
      this.send({ kind: "sync.ack", payload: { cursor: envelope.sync_cursor } }).catch(() => undefined);
    }
  }
}
