// A scripted stand-in for the browser `WebSocket` used by the relay tests.
// Tests open, feed and close it explicitly; nothing here talks to a network.

export class FakeWebSocket {
  static instances: FakeWebSocket[] = [];

  static reset(): void {
    FakeWebSocket.instances = [];
  }

  static create(url: string): WebSocket {
    return new FakeWebSocket(url) as unknown as WebSocket;
  }

  readyState = 0;
  readonly sent: string[] = [];
  readonly closeCalls: { code?: number; reason?: string }[] = [];
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;

  constructor(readonly url: string) {
    FakeWebSocket.instances.push(this);
  }

  send(data: string): void {
    if (this.readyState !== 1) throw new Error("FakeWebSocket is not open");
    this.sent.push(String(data));
  }

  close(code?: number, reason?: string): void {
    if (this.readyState === 3) return;
    this.readyState = 3;
    this.closeCalls.push({ code, reason });
    this.onclose?.({ code: code ?? 1000, reason: reason ?? "", wasClean: true } as CloseEvent);
  }

  /** The server accepted the connection. */
  open(): void {
    this.readyState = 1;
    this.onopen?.(new Event("open"));
  }

  /** The server sent a text frame. */
  receive(data: unknown): void {
    this.onmessage?.({
      data: typeof data === "string" ? data : JSON.stringify(data),
    } as MessageEvent);
  }

  /** The server or the network closed the connection. */
  drop(code: number, reason = ""): void {
    this.readyState = 3;
    this.onclose?.({ code, reason, wasClean: false } as CloseEvent);
  }
}
