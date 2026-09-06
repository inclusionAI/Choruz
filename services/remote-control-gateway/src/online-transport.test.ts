import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomBytes, randomUUID, createCipheriv, createDecipheriv } from "node:crypto";
import { fileURLToPath, URL as NodeURL } from "node:url";
import { Buffer } from "node:buffer";
import { afterAll, beforeAll, expect, it } from "vitest";
import { getPlatformProxy, unstable_dev, type Unstable_DevWorker } from "wrangler";
import WebSocket from "ws";

const root = fileURLToPath(new NodeURL("../", import.meta.url));
let worker: Unstable_DevWorker;
let state: string;
let origin: string;
const sockets: WebSocket[] = [];
beforeAll(async () => {
  state = await mkdtemp(join(tmpdir(), "choruz-online-transport-"));
  const platform = await getPlatformProxy<{ ONLINE_DATABASE: D1Database }>({ configPath: join(root, "wrangler.toml"), persist: { path: join(state, "v3") }, remoteBindings: false });
  try {
    for (const file of (await readdir(join(root, "migrations"))).filter(name => name.endsWith(".sql")).sort()) {
      await platform.env.ONLINE_DATABASE.exec(await readFile(join(root, "migrations", file), "utf8"));
    }
  } finally { await platform.dispose(); }
  const secret = Buffer.from(randomBytes(32)).toString("hex");
  worker = await unstable_dev(join(root, "src/index.ts"), { config: join(root, "wrangler.toml"), ip: "127.0.0.1", port: 0, inspectorPort: 0, persist: true, persistTo: state, vars: { ONLINE_AUTH_SECRET: secret, GATEWAY_AUTH_SECRET: secret }, logLevel: "error", experimental: { disableExperimentalWarning: true, disableDevRegistry: true, watch: false } });
  origin = `http://127.0.0.1:${worker.port}`;
}, 60_000);
afterAll(async () => {
  for (const socket of sockets) socket.terminate();
  await worker?.stop();
  if (state) await rm(state, { recursive: true, force: true });
});

type Account = { token: string; user: { id: string } };
async function request(path: string, account?: Account, method = "GET", body?: object, extra?: Record<string, string>) {
  return fetch(`${origin}/v1/online/${path}`, { method, headers: { origin, "content-type": "application/json", ...(account ? { authorization: `Bearer ${account.token}` } : {}), ...extra }, body: body ? JSON.stringify(body) : undefined });
}
async function account(): Promise<Account> {
  const response = await request("auth/sign-up/email", undefined, "POST", { email: `${randomUUID()}@example.test`, name: "Transport test", password: Buffer.from(randomBytes(20)).toString("hex") });
  expect(response.status).toBe(200);
  return await response.json() as Account;
}
async function connect(account: Account) {
  const socket = new WebSocket(`${origin.replace("http:", "ws:")}/v1/online/connect`, { headers: { authorization: `Bearer ${account.token}` } });
  sockets.push(socket);
  const queue: Record<string, unknown>[] = [];
  let wake: (() => void) | undefined;
  socket.on("message", data => { queue.push(JSON.parse(data.toString())); wake?.(); });
  const next = async () => {
    if (!queue.length) await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => { wake = undefined; reject(new Error("Expected Online frame was not delivered")); }, 5000);
      wake = () => { clearTimeout(timer); wake = undefined; resolve(); };
    });
    return queue.shift()!;
  };
  expect(await next()).toMatchObject({ kind: "ready", account_id: account.user.id });
  return { socket, next, send: (frame: object) => socket.send(JSON.stringify(frame)) };
}

it("relays real ciphertext only between invited accounts, replays unacknowledged deliveries and enforces revocation", async () => {
  const [alice, bob, mallory] = await Promise.all([account(), account(), account()]);
  expect((await request("links", undefined, "POST")).status).toBe(401);
  const created = await request("links", alice, "POST");
  expect(created.status).toBe(200);
  const link = await created.json() as { id: string; invitation: string };
  expect((await request(`links/${link.id}/join`, bob, "POST", undefined, { "x-online-invitation": "0".repeat(64) })).status).toBe(403);
  expect((await request(`links/${link.id}/join`, bob, "POST", undefined, { "x-online-invitation": link.invitation })).status).toBe(200);
  expect((await request(`links/${link.id}/join`, bob, "POST", undefined, { "x-online-invitation": link.invitation })).status).toBe(200);
  expect((await request(`links/${link.id}/join`, mallory, "POST", undefined, { "x-online-invitation": link.invitation })).status).toBe(403);
  expect((await request(`links/${link.id}`, mallory)).status).toBe(404);
  const a = await connect(alice);
  const m = await connect(mallory);
  const key = randomBytes(32), iv = randomBytes(12);
  const cipher = createCipheriv("aes-256-gcm", key, iv);
  const plaintext = "ordinary private group message";
  const bytes = Buffer.concat([cipher.update(plaintext), cipher.final(), cipher.getAuthTag()]);
  const frame = { kind: "send", id: randomUUID(), channel: link.id, recipient: bob.user.id, envelope: { kind: "e2e", iv: Buffer.from(iv).toString("base64url"), ciphertext: bytes.toString("base64url") } };
  m.send(frame);
  expect(await m.next()).toMatchObject({ kind: "rejected", reason: "link_unavailable" });
  a.send({ ...frame, envelope: { kind: "message", content: plaintext } });
  expect(await a.next()).toMatchObject({ kind: "error" });
  a.send(frame);
  expect(await a.next()).toMatchObject({ kind: "accepted", id: frame.id });
  const otherCreated = await request("links", mallory, "POST");
  expect(otherCreated.status).toBe(200);
  const otherLink = await otherCreated.json() as { id: string; invitation: string };
  expect((await request(`links/${otherLink.id}/join`, bob, "POST", undefined, { "x-online-invitation": otherLink.invitation })).status).toBe(200);
  m.send({ ...frame, channel: otherLink.id });
  expect(await m.next()).toMatchObject({ kind: "rejected", id: frame.id });
  const b = await connect(bob);
  const delivered = await b.next();
  expect(delivered).toMatchObject({ kind: "delivery", sender: alice.user.id, envelope: frame.envelope });
  expect(JSON.stringify(delivered)).not.toContain(plaintext);
  const decipher = createDecipheriv("aes-256-gcm", key, iv);
  decipher.setAuthTag(bytes.subarray(-16));
  expect(Buffer.concat([decipher.update(bytes.subarray(0, -16)), decipher.final()]).toString()).toBe(plaintext);
  b.socket.terminate();
  const resumed = await connect(bob);
  expect(await resumed.next()).toMatchObject({ kind: "delivery", id: frame.id });
  resumed.send({ kind: "ack", id: frame.id });
  resumed.send({ kind: "ping" });
  expect(await resumed.next()).toMatchObject({ kind: "pong" });
  resumed.socket.terminate();
  const acknowledged = await connect(bob);
  acknowledged.send({ kind: "ping" });
  expect(await acknowledged.next()).toMatchObject({ kind: "pong" });
  expect((await request(`links/${link.id}`, alice, "DELETE")).status).toBe(204);
  expect(await a.next()).toMatchObject({ kind: "revoked", channel: link.id });
  expect(await acknowledged.next()).toMatchObject({ kind: "revoked", channel: link.id });
  a.send({ ...frame, id: randomUUID() });
  expect(await a.next()).toMatchObject({ kind: "rejected", reason: "link_unavailable" });
  expect((await request("auth/sign-out", alice, "POST", {})).status).toBe(200);
  const closed = new Promise<number>(resolve => a.socket.once("close", resolve));
  a.send({ kind: "ping" });
  expect(await closed).toBe(4001);
}, 30_000);
