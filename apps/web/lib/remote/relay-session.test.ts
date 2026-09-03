import { beforeEach, describe, expect, it, vi } from "vitest";

import { FakeWebSocket } from "./fake-web-socket";
import { decryptWithSessionKey, encryptWithSessionKey } from "./remote-control-crypto";
import { RelaySession, type RelayEnvelope, type RelayStatus, gatewayConnectUrl } from "./relay-session";

const credentials = {
  device_id: "device-1",
  gateway_url: "https://gateway.example",
  gateway_ticket: "stable-ticket",
  session_key: "k".repeat(43),
};

async function decryptFrame(raw: string): Promise<RelayEnvelope> {
  const value = JSON.parse(raw) as { kind: string; iv: string; ciphertext: string };
  expect(value.kind).toBe("e2e");
  return decryptWithSessionKey<RelayEnvelope>(credentials.session_key, value.iv, value.ciphertext);
}

async function encryptFrame(envelope: RelayEnvelope): Promise<string> {
  return JSON.stringify({ kind: "e2e", ...await encryptWithSessionKey(credentials.session_key, envelope) });
}

async function connectedSession() {
  const statuses: RelayStatus[] = [];
  const received: RelayEnvelope[] = [];
  const session = new RelaySession(credentials, {
    createSocket: FakeWebSocket.create,
    reconnectDelayMs: 5,
  });
  session.onStatus((status) => statuses.push(status));
  session.subscribe((envelope) => received.push(envelope));
  session.start();
  const rendezvous = FakeWebSocket.instances[0];
  rendezvous.open();
  rendezvous.receive({
    kind: "session.offer",
    payload: { session_id: "s1", gateway_url: "https://edge.example", gateway_ticket: "transport-ticket" },
  });
  const transport = FakeWebSocket.instances[1];
  transport.open();
  await vi.waitFor(() => expect(session.status).toBe("connected"));
  return { session, statuses, received, rendezvous, transport };
}

describe("gatewayConnectUrl", () => {
  it("targets /connect over the socket scheme", () => {
    expect(gatewayConnectUrl("https://gateway.example/x?y", { ticket: "t 1" }))
      .toBe("wss://gateway.example/connect?ticket=t+1");
    expect(gatewayConnectUrl("http://127.0.0.1:8787", { code: "12345678", role: "pair_client" }))
      .toBe("ws://127.0.0.1:8787/connect?code=12345678&role=pair_client");
  });
});

describe("RelaySession", () => {
  beforeEach(() => FakeWebSocket.reset());

  it("waits on the rendezvous room, follows the transport offer and greets the host", async () => {
    const { statuses, rendezvous, transport } = await connectedSession();
    expect(rendezvous.url).toBe("wss://gateway.example/connect?ticket=stable-ticket");
    expect(transport.url).toBe("wss://edge.example/connect?ticket=transport-ticket");
    expect(statuses).toEqual(["connecting", "waiting", "connected"]);
    expect(transport.sent).toHaveLength(1);
    await expect(decryptFrame(transport.sent[0])).resolves.toEqual({
      kind: "device.hello",
      payload: { device_id: "device-1" },
    });
  });

  it("decrypts host frames in order and acknowledges projected sync changes", async () => {
    const { session, received, transport } = await connectedSession();
    transport.receive(await encryptFrame({ kind: "stream.opened", payload: { stream_id: "s" } }));
    transport.receive(await encryptFrame({ kind: "message", payload: { content: "hi" }, sync_cursor: 7 }));
    transport.receive("not json");
    transport.receive({ type: "gateway.pong" });
    await vi.waitFor(() => expect(received).toHaveLength(2));
    expect(received.map((envelope) => envelope.kind)).toEqual(["stream.opened", "message"]);
    await vi.waitFor(() => expect(transport.sent).toHaveLength(2));
    await expect(decryptFrame(transport.sent[1])).resolves.toEqual({
      kind: "sync.ack",
      payload: { cursor: 7 },
    });
    expect(session.status).toBe("connected");
  });

  it("sends envelopes in call order and rejects when no transport is open", async () => {
    const { session, transport } = await connectedSession();
    await Promise.all([
      session.send({ kind: "http.request", payload: { request_id: "a" } }),
      session.send({ kind: "http.body", payload: { request_id: "a", index: 0 } }),
      session.send({ kind: "http.body", payload: { request_id: "a", index: 1 } }),
    ]);
    const kinds = await Promise.all(transport.sent.slice(1).map(decryptFrame));
    expect(kinds.map((envelope) => `${envelope.kind}:${envelope.payload?.index ?? ""}`))
      .toEqual(["http.request:", "http.body:0", "http.body:1"]);

    transport.drop(1006);
    expect(session.status).toBe("waiting");
    await expect(session.send({ kind: "http.request" })).rejects.toThrow("not connected");
  });

  it("replaces the transport when the host offers a new room", async () => {
    const { session, rendezvous, transport } = await connectedSession();
    rendezvous.receive({
      kind: "session.offer",
      payload: { session_id: "s2", gateway_url: "https://edge2.example", gateway_ticket: "t2" },
    });
    expect(transport.closeCalls).toEqual([{ code: 1000, reason: "Transport rotated" }]);
    const next = FakeWebSocket.instances[2];
    expect(next.url).toBe("wss://edge2.example/connect?ticket=t2");
    expect(session.status).toBe("waiting");
    next.open();
    await vi.waitFor(() => expect(session.status).toBe("connected"));
  });

  it("reconnects the rendezvous room after a drop and stops when revoked", async () => {
    const { session, statuses, rendezvous } = await connectedSession();
    rendezvous.drop(1006);
    expect(session.status).toBe("connecting");
    await vi.waitFor(() => expect(FakeWebSocket.instances).toHaveLength(3));
    const again = FakeWebSocket.instances[2];
    expect(again.url).toBe(rendezvous.url);
    again.open();
    // The transport is still connected, so the session is back to connected.
    expect(session.status).toBe("connected");

    again.drop(4003, "Device revoked");
    expect(session.status).toBe("revoked");
    await new Promise((resolve) => setTimeout(resolve, 15));
    expect(FakeWebSocket.instances).toHaveLength(3);
    expect(statuses.at(-1)).toBe("revoked");
  });

  it("close() ends both sockets and stops reconnecting", async () => {
    const { session, rendezvous, transport } = await connectedSession();
    session.close();
    expect(session.status).toBe("closed");
    expect(rendezvous.closeCalls).toEqual([{ code: 1000, reason: "Session closed" }]);
    expect(transport.closeCalls).toEqual([{ code: 1000, reason: "Session closed" }]);
    await new Promise((resolve) => setTimeout(resolve, 15));
    expect(FakeWebSocket.instances).toHaveLength(2);
  });
});
