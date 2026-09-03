import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { FakeWebSocket } from "./fake-web-socket";
import {
  clearRemoteCredentials,
  loadRemoteCredentials,
  pairWithHost,
  storeRemoteCredentials,
} from "./relay-pairing";
import {
  createPairingKey,
  createPairingNonce,
  derivePairingCommitment,
  derivePairingProof,
  derivePairingSecret,
  encryptWithSessionKey,
  exportPairingPublicKey,
} from "./remote-control-crypto";

type Frame = Record<string, unknown>;
const PAIRING_ID = "AAAAAAAAAAAAAAAAAAAAAA";
const PAIRING_SECRET = "BBBBBBBBBBBBBBBBBBBBBB";
const CREDENTIAL = `v1.${PAIRING_ID}.${PAIRING_SECRET}`;

const parse = (raw: string) => JSON.parse(raw) as Frame;

/** Plays the host's half of the handshake the way remote-control-manager.tsx does. */
async function hostKeys() {
  const keys = await createPairingKey();
  const publicKey = await exportPairingPublicKey(keys.publicKey);
  const nonce = createPairingNonce();
  return { keys, publicKey, nonce, commitment: await derivePairingCommitment(publicKey, nonce) };
}

describe("pairWithHost", () => {
  beforeEach(() => FakeWebSocket.reset());

  it("commits, reveals and receives the encrypted credentials", async () => {
    const pairing = pairWithHost({
      gatewayUrl: "https://gateway.example",
      credential: CREDENTIAL,
      deviceName: "  Phone ",
      createSocket: FakeWebSocket.create,
    });
    await vi.waitFor(() => expect(FakeWebSocket.instances).toHaveLength(1));
    const socket = FakeWebSocket.instances[0];
    expect(socket.url).toBe(`wss://gateway.example/connect?pairing_id=${PAIRING_ID}&role=pair_client`);
    expect(socket.url).not.toContain(PAIRING_SECRET);
    socket.open();
    const commit = parse(socket.sent[0]);
    expect(commit).toMatchObject({ kind: "pair.commit", device_name: "Phone" });
    expect(typeof commit.device_commitment).toBe("string");

    const host = await hostKeys();
    socket.receive({ type: "gateway.ready" });
    socket.receive({ kind: "pair.commit", host_commitment: host.commitment });
    await vi.waitFor(() => expect(socket.sent).toHaveLength(2));
    const reveal = parse(socket.sent[1]);
    expect(reveal.kind).toBe("pair.reveal");
    const devicePublicKey = reveal.device_public_key as string;
    await expect(derivePairingCommitment(devicePublicKey, reveal.device_nonce as string))
      .resolves.toBe(commit.device_commitment);

    const hostSecret = await derivePairingSecret(host.keys.privateKey, devicePublicKey, PAIRING_SECRET);
    socket.receive({
      kind: "pair.reveal",
      host_public_key: host.publicKey,
      host_nonce: host.nonce,
      host_proof: await derivePairingProof(hostSecret, "host", host.publicKey, devicePublicKey),
    });
    await vi.waitFor(() => expect(socket.sent).toHaveLength(3));
    await expect(derivePairingProof(hostSecret, "device", host.publicKey, devicePublicKey))
      .resolves.toBe(parse(socket.sent[2]).device_proof);

    const credentials = {
      device_id: "device-1",
      gateway_url: "https://gateway.example",
      gateway_ticket: "ticket-1",
      session_key: "s".repeat(43),
    };
    socket.receive({ kind: "pair.complete", ...await encryptWithSessionKey(hostSecret, credentials) });
    await expect(pairing).resolves.toEqual(credentials);
    expect(socket.closeCalls).toEqual([{ code: 1000, reason: "Pairing complete" }]);
  });

  it("falls back to the pairing secret when the host sends no session key", async () => {
    const pairing = pairWithHost({
      gatewayUrl: "https://gateway.example",
      credential: CREDENTIAL,
      deviceName: "Phone",
      createSocket: FakeWebSocket.create,
    });
    await vi.waitFor(() => expect(FakeWebSocket.instances).toHaveLength(1));
    const socket = FakeWebSocket.instances[0];
    socket.open();
    const host = await hostKeys();
    socket.receive({ kind: "pair.commit", host_commitment: host.commitment });
    await vi.waitFor(() => expect(socket.sent).toHaveLength(2));
    const devicePublicKey = parse(socket.sent[1]).device_public_key as string;
    const hostSecret = await derivePairingSecret(host.keys.privateKey, devicePublicKey, PAIRING_SECRET);
    socket.receive({ kind: "pair.reveal", host_public_key: host.publicKey, host_nonce: host.nonce,
      host_proof: await derivePairingProof(hostSecret, "host", host.publicKey, devicePublicKey) });
    await new Promise((resolve) => setTimeout(resolve, 20));
    socket.receive({
      kind: "pair.complete",
      ...await encryptWithSessionKey(hostSecret, {
        device_id: "d", gateway_url: "https://gateway.example", gateway_ticket: "t",
      }),
    });
    await expect(pairing).resolves.toMatchObject({ session_key: hostSecret });
  });

  it("rejects a host whose reveal does not match its commitment", async () => {
    const pairing = pairWithHost({
      gatewayUrl: "https://gateway.example",
      credential: CREDENTIAL,
      deviceName: "Phone",
      createSocket: FakeWebSocket.create,
    });
    await vi.waitFor(() => expect(FakeWebSocket.instances).toHaveLength(1));
    const socket = FakeWebSocket.instances[0];
    socket.open();
    const host = await hostKeys();
    socket.receive({ kind: "pair.commit", host_commitment: host.commitment });
    await vi.waitFor(() => expect(socket.sent).toHaveLength(2));
    socket.receive({ kind: "pair.reveal", host_public_key: host.publicKey, host_nonce: "different", host_proof: "x" });
    await expect(pairing).rejects.toThrow("Host pairing commitment did not match.");
    expect(socket.closeCalls).toEqual([{ code: 4002, reason: "Pairing failed" }]);
  });

  it("rejects a host that does not know the credential secret", async () => {
    const pairing = pairWithHost({
      gatewayUrl: "https://gateway.example",
      credential: CREDENTIAL,
      deviceName: "Phone",
      createSocket: FakeWebSocket.create,
    });
    await vi.waitFor(() => expect(FakeWebSocket.instances).toHaveLength(1));
    const socket = FakeWebSocket.instances[0];
    socket.open();
    const host = await hostKeys();
    socket.receive({ kind: "pair.commit", host_commitment: host.commitment });
    await vi.waitFor(() => expect(socket.sent).toHaveLength(2));
    socket.receive({
      kind: "pair.reveal",
      host_public_key: host.publicKey,
      host_nonce: host.nonce,
      host_proof: "invalid-proof",
    });
    await expect(pairing).rejects.toThrow("Host did not prove possession of the pairing credential.");
  });

  it("rejects a completion that arrives before the transcript was verified", async () => {
    const pairing = pairWithHost({
      gatewayUrl: "https://gateway.example",
      credential: CREDENTIAL,
      deviceName: "Phone",
      createSocket: FakeWebSocket.create,
    });
    await vi.waitFor(() => expect(FakeWebSocket.instances).toHaveLength(1));
    const socket = FakeWebSocket.instances[0];
    socket.open();
    socket.receive({ kind: "pair.complete", iv: "AAAAAAAAAAAAAAAA", ciphertext: "AAAA" });
    await expect(pairing).rejects.toThrow("Pairing transcript was not verified.");
  });

  it("surfaces the gateway's close reason and a timeout", async () => {
    const closed = pairWithHost({
      gatewayUrl: "https://gateway.example",
      credential: CREDENTIAL,
      deviceName: "Phone",
      createSocket: FakeWebSocket.create,
    });
    await vi.waitFor(() => expect(FakeWebSocket.instances).toHaveLength(1));
    FakeWebSocket.instances[0].drop(4004, "Pairing credential is invalid, expired, or not ready");
    await expect(closed).rejects.toThrow("Pairing credential is invalid, expired, or not ready");

    const slow = pairWithHost({
      gatewayUrl: "https://gateway.example",
      credential: CREDENTIAL,
      deviceName: "Phone",
      createSocket: FakeWebSocket.create,
      timeoutMs: 10,
    });
    await expect(slow).rejects.toThrow("Pairing handshake timed out.");
  });
});

describe("remote credential storage", () => {
  const store = new Map<string, string>();

  beforeEach(() => {
    store.clear();
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, value); },
      removeItem: (key: string) => { store.delete(key); },
    });
  });

  afterEach(() => vi.unstubAllGlobals());

  it("keeps credentials per gateway origin and drops malformed ones", () => {
    const credentials = {
      device_id: "d", gateway_url: "https://gateway.example", gateway_ticket: "t", session_key: "k",
    };
    storeRemoteCredentials("https://gateway.example/remote?x", credentials);
    expect([...store.keys()]).toEqual(["choruz.remote-control.credentials:https://gateway.example"]);
    expect(loadRemoteCredentials("https://gateway.example")).toEqual(credentials);
    expect(loadRemoteCredentials("https://other.example")).toBeNull();

    store.set("choruz.remote-control.credentials:https://gateway.example", "{\"device_id\":1}");
    expect(loadRemoteCredentials("https://gateway.example")).toBeNull();

    storeRemoteCredentials("https://gateway.example", credentials);
    clearRemoteCredentials("https://gateway.example");
    expect(loadRemoteCredentials("https://gateway.example")).toBeNull();
  });
});
