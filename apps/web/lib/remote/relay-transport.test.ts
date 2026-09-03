import { describe, expect, it, vi } from "vitest";

import { base64UrlToBytes, bytesToBase64Url } from "./remote-control-crypto";
import type { RelayEnvelope, RelayLink, RelayStatus } from "./relay-session";
import { CHUNK_BYTES, createRelayTransport, relayPath } from "./relay-transport";

class FakeLink implements RelayLink {
  status: RelayStatus = "connected";
  readonly sent: RelayEnvelope[] = [];
  private readonly handlers = new Set<(envelope: RelayEnvelope) => void>();
  private readonly statusHandlers = new Set<(status: RelayStatus) => void>();
  failSends = false;

  async send(envelope: RelayEnvelope): Promise<void> {
    if (this.failSends) throw new Error("The paired computer is not connected.");
    this.sent.push(envelope);
  }

  subscribe(handler: (envelope: RelayEnvelope) => void): () => void {
    this.handlers.add(handler);
    return () => this.handlers.delete(handler);
  }

  onStatus(handler: (status: RelayStatus) => void): () => void {
    this.statusHandlers.add(handler);
    return () => this.statusHandlers.delete(handler);
  }

  close(): void {}

  emit(envelope: RelayEnvelope): void {
    for (const handler of this.handlers) handler(envelope);
  }

  setStatus(status: RelayStatus): void {
    this.status = status;
    for (const handler of this.statusHandlers) handler(status);
  }
}

const payloadOf = (envelope: RelayEnvelope) => envelope.payload as Record<string, unknown>;

async function flush(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

describe("relayPath", () => {
  it("keeps same-origin paths and strips the origin of absolute URLs", () => {
    expect(relayPath("/api/v1/bootstrap?since=1")).toBe("/api/v1/bootstrap?since=1");
    expect(relayPath("http://localhost:3100/api/agents?x=1")).toBe("/api/agents?x=1");
  });
});

describe("createRelayTransport fetch", () => {
  it("sends a GET as one http.request and assembles the response", async () => {
    const link = new FakeLink();
    const transport = createRelayTransport(link);
    const pending = transport.fetch("/api/v1/bootstrap", {
      headers: { Authorization: "Bearer local", "X-Trace-Id": "t1" },
    });
    await flush();
    expect(link.sent).toHaveLength(1);
    const request = payloadOf(link.sent[0]);
    expect(link.sent[0].kind).toBe("http.request");
    expect(request.method).toBe("GET");
    expect(request.path).toBe("/api/v1/bootstrap");
    expect(request.body_chunks).toBe(0);
    expect(request.headers).toEqual({ authorization: "Bearer local", "x-trace-id": "t1" });

    const requestId = request.request_id as string;
    const body = new TextEncoder().encode("{\"ok\":true}");
    link.emit({
      kind: "http.response",
      payload: {
        request_id: requestId,
        status: 200,
        headers: { "content-type": "application/json" },
        body_chunks: 1,
      },
    });
    link.emit({
      kind: "http.body",
      payload: { request_id: requestId, index: 0, data: bytesToBase64Url(body) },
    });
    const response = await pending;
    expect(response.status).toBe(200);
    expect(response.headers.get("content-type")).toBe("application/json");
    await expect(response.json()).resolves.toEqual({ ok: true });
    transport.dispose();
  });

  it("chunks a large POST body and reassembles chunked responses in index order", async () => {
    const link = new FakeLink();
    const transport = createRelayTransport(link);
    const bytes = new Uint8Array(CHUNK_BYTES * 2 + 5).map((_, index) => index % 251);
    const pending = transport.fetch("/api/v1/big", { method: "post", body: bytes });
    await flush();
    expect(link.sent.map((envelope) => envelope.kind)).toEqual([
      "http.request", "http.body", "http.body", "http.body",
    ]);
    const head = payloadOf(link.sent[0]);
    expect(head.method).toBe("POST");
    expect(head.body_chunks).toBe(3);
    const sentBytes = link.sent.slice(1).flatMap((envelope) => [
      ...new Uint8Array(base64UrlToBytes(payloadOf(envelope).data as string)),
    ]);
    expect(sentBytes).toEqual([...bytes]);
    expect(payloadOf(link.sent[1]).index).toBe(0);
    expect(payloadOf(link.sent[3]).index).toBe(2);

    const requestId = head.request_id as string;
    link.emit({
      kind: "http.response",
      payload: { request_id: requestId, status: 200, headers: {}, body_chunks: 2 },
    });
    link.emit({
      kind: "http.body",
      payload: { request_id: requestId, index: 1, data: bytesToBase64Url(new Uint8Array([4, 5])) },
    });
    link.emit({
      kind: "http.body",
      payload: { request_id: requestId, index: 0, data: bytesToBase64Url(new Uint8Array([1, 2, 3])) },
    });
    const response = await pending;
    expect([...new Uint8Array(await response.arrayBuffer())]).toEqual([1, 2, 3, 4, 5]);
    transport.dispose();
  });

  it("gives string bodies the fetch default content type and keeps an explicit one", async () => {
    const link = new FakeLink();
    const transport = createRelayTransport(link);
    void transport.fetch("/api/a", { method: "POST", body: "hello" }).catch(() => undefined);
    void transport.fetch("/api/b", {
      method: "POST",
      body: "{}",
      headers: { "content-type": "application/json" },
    }).catch(() => undefined);
    await flush();
    const requests = link.sent.filter((envelope) => envelope.kind === "http.request").map(payloadOf);
    expect(requests.map((request) => request.headers)).toEqual([
      { "content-type": "text/plain;charset=UTF-8" },
      { "content-type": "application/json" },
    ]);
    transport.dispose();
  });

  it("rejects like a network failure when the host reports an error or the link drops", async () => {
    const link = new FakeLink();
    const transport = createRelayTransport(link);
    const failed = transport.fetch("/api/v1/x");
    await flush();
    link.emit({
      kind: "http.response",
      payload: { request_id: payloadOf(link.sent[0]).request_id, error: "connection refused" },
    });
    await expect(failed).rejects.toThrow(TypeError);
    await expect(failed).rejects.toThrow("connection refused");

    const dropped = transport.fetch("/api/v1/y");
    await flush();
    link.setStatus("waiting");
    await expect(dropped).rejects.toThrow("not connected");

    link.setStatus("connected");
    link.failSends = true;
    await expect(transport.fetch("/api/v1/z")).rejects.toThrow("not connected");
    transport.dispose();
  });

  it("times out a request the host never answers", async () => {
    vi.useFakeTimers();
    try {
      const link = new FakeLink();
      const transport = createRelayTransport(link, { requestTimeoutMs: 50 });
      const pending = transport.fetch("/api/v1/slow");
      const rejection = expect(pending).rejects.toThrow("did not respond in time");
      await vi.advanceTimersByTimeAsync(60);
      await rejection;
      transport.dispose();
    } finally {
      vi.useRealTimers();
    }
  });

  it("returns a null body for statuses that forbid one", async () => {
    const link = new FakeLink();
    const transport = createRelayTransport(link);
    const pending = transport.fetch("/api/v1/none", { method: "DELETE" });
    await flush();
    link.emit({
      kind: "http.response",
      payload: { request_id: payloadOf(link.sent[0]).request_id, status: 204, headers: {}, body_chunks: 0 },
    });
    const response = await pending;
    expect(response.status).toBe(204);
    expect(response.body).toBeNull();
    transport.dispose();
  });
});

describe("createRelayTransport socket", () => {
  it("opens, receives chunked text and binary, sends, and closes over stream frames", async () => {
    const link = new FakeLink();
    const transport = createRelayTransport(link);
    const socket = transport.socket("/v1/ws/terminals/b1?cols=80&rows=24");
    socket.binaryType = "arraybuffer";
    const events: string[] = [];
    const messages: (string | ArrayBuffer)[] = [];
    socket.onopen = () => events.push("open");
    socket.onmessage = (event) => messages.push(event.data as string | ArrayBuffer);
    socket.onclose = (event) => events.push(`close ${event.code} ${event.reason}`);
    await flush();
    expect(link.sent[0].kind).toBe("stream.open");
    const streamId = payloadOf(link.sent[0]).stream_id as string;
    expect(payloadOf(link.sent[0]).path).toBe("/v1/ws/terminals/b1?cols=80&rows=24");
    expect(socket.readyState).toBe(0);
    expect(() => socket.send("x")).toThrow("not open");

    link.emit({ kind: "stream.opened", payload: { stream_id: streamId } });
    expect(socket.readyState).toBe(1);
    expect(events).toEqual(["open"]);

    link.emit({
      kind: "stream.data",
      payload: { stream_id: streamId, encoding: "text", data: bytesToBase64Url(new TextEncoder().encode("hel")), last: false },
    });
    link.emit({
      kind: "stream.data",
      payload: { stream_id: streamId, encoding: "text", data: bytesToBase64Url(new TextEncoder().encode("lo")), last: true },
    });
    link.emit({
      kind: "stream.data",
      payload: { stream_id: streamId, encoding: "binary", data: bytesToBase64Url(new Uint8Array([7, 8, 9])), last: true },
    });
    expect(messages[0]).toBe("hello");
    expect([...new Uint8Array(messages[1] as ArrayBuffer)]).toEqual([7, 8, 9]);

    socket.send("{\"type\":\"resize\"}");
    socket.send(new Uint8Array(CHUNK_BYTES + 1));
    await flush();
    const dataFrames = link.sent.filter((envelope) => envelope.kind === "stream.data").map(payloadOf);
    expect(dataFrames).toHaveLength(3);
    expect(dataFrames[0]).toMatchObject({ stream_id: streamId, encoding: "text", last: true });
    expect(new TextDecoder().decode(base64UrlToBytes(dataFrames[0].data as string))).toBe("{\"type\":\"resize\"}");
    expect(dataFrames[1]).toMatchObject({ encoding: "binary", last: false });
    expect(dataFrames[2]).toMatchObject({ encoding: "binary", last: true });
    expect(base64UrlToBytes(dataFrames[1].data as string).byteLength).toBe(CHUNK_BYTES);
    expect(base64UrlToBytes(dataFrames[2].data as string).byteLength).toBe(1);

    socket.close(1000, "done");
    await flush();
    expect(socket.readyState).toBe(3);
    expect(link.sent.at(-1)).toEqual({
      kind: "stream.close",
      payload: { stream_id: streamId, code: 1000, reason: "done" },
    });
    expect(events).toEqual(["open", "close 1000 done"]);
    transport.dispose();
  });

  it("closes the socket when the host closes the stream or the link drops", async () => {
    const link = new FakeLink();
    const transport = createRelayTransport(link);
    const hostClosed = transport.socket("/v1/ws/sync");
    const linkDropped = transport.socket("/v1/ws/sync");
    const closes: string[] = [];
    let errors = 0;
    hostClosed.onclose = (event) => closes.push(`host ${event.code} ${event.reason}`);
    linkDropped.onclose = (event) => closes.push(`link ${event.code}`);
    linkDropped.onerror = () => { errors += 1; };
    await flush();
    const hostStreamId = payloadOf(link.sent[0]).stream_id as string;
    link.emit({ kind: "stream.opened", payload: { stream_id: hostStreamId } });
    link.emit({ kind: "stream.close", payload: { stream_id: hostStreamId, code: 4004, reason: "rotated" } });
    expect(hostClosed.readyState).toBe(3);

    link.setStatus("connecting");
    expect(linkDropped.readyState).toBe(3);
    expect(errors).toBe(1);
    expect(closes).toEqual(["host 4004 rotated", "link 1006"]);
    transport.dispose();
  });
});
