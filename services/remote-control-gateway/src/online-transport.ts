import { DurableObject } from "cloudflare:workers";
import { onlineAuth, type OnlineAuthEnv } from "./online-auth";

export interface OnlineTransportEnv extends OnlineAuthEnv {
  ONLINE_MAILBOXES: DurableObjectNamespace<OnlineMailbox>;
}

type Session = { account: string; session: string; expires: number; window?: number; count?: number };
type Link = { id: string; owner_id: string; peer_id: string | null };
type Envelope = { kind: "e2e"; iv: string; ciphertext: string };
type Delivery = { id: string; channel: string; sender: string; recipient: string; envelope: Envelope; created: number };
const RETENTION_MS = 7 * 24 * 60 * 60 * 1000;
const MAX_PENDING = 256;
const MAX_FRAME = 1_000_000;

function transportLog(phase: string, account: string, channel?: string) {
  console.log(JSON.stringify({ event: "online_transport", phase, account_id: account, ...(channel ? { channel } : {}) }));
}

function mailbox(env: OnlineTransportEnv, account: string) {
  return env.ONLINE_MAILBOXES.get(env.ONLINE_MAILBOXES.idFromName(account));
}

async function identity(request: Request, env: OnlineTransportEnv): Promise<Session | null> {
  if (!env.ONLINE_DATABASE || !env.ONLINE_AUTH_SECRET) return null;
  const session = await onlineAuth(env.ONLINE_DATABASE, env.ONLINE_AUTH_SECRET, new URL(request.url).origin)
    .api.getSession({ headers: request.headers });
  return session ? { account: session.user.id, session: session.session.id, expires: session.session.expiresAt.getTime() } : null;
}

async function digest(value: string): Promise<string> {
  return Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value))), b => b.toString(16).padStart(2, "0")).join("");
}

export function encryptedEnvelope(value: unknown): value is Envelope {
  if (!value || typeof value !== "object") return false;
  const frame = value as Partial<Envelope>;
  return frame.kind === "e2e" && typeof frame.iv === "string" && /^[A-Za-z0-9_-]{16}$/.test(frame.iv)
    && typeof frame.ciphertext === "string" && /^[A-Za-z0-9_-]{22,900000}$/.test(frame.ciphertext);
}

/** These grants authorize ciphertext delivery, never device-control requests. */
export async function onlineTransportResponse(request: Request, env: OnlineTransportEnv): Promise<Response> {
  const actor = await identity(request, env);
  if (!actor) return new Response("Online sign-in required", { status: 401 });
  const url = new URL(request.url);
  const db = env.ONLINE_DATABASE!;
  if (url.pathname === "/v1/online/connect" && request.headers.get("upgrade")?.toLowerCase() === "websocket") {
    const headers = new Headers(request.headers);
    headers.set("x-online-session", JSON.stringify(actor));
    return mailbox(env, actor.account).fetch(new Request(request, { headers }));
  }
  if (url.pathname === "/v1/online/links" && request.method === "POST") {
    const count = await db.prepare("SELECT COUNT(*) AS count FROM online_link WHERE owner_id=?").bind(actor.account).first<{ count: number }>();
    if ((count?.count ?? 0) >= 128) return new Response("Too many Online links", { status: 429 });
    const id = crypto.randomUUID();
    const invitation = crypto.randomUUID().replaceAll("-", "") + crypto.randomUUID().replaceAll("-", "");
    const expires = Date.now() + 24 * 60 * 60 * 1000;
    await db.prepare("INSERT INTO online_link(id,owner_id,invitation_hash,expires_at,created_at) VALUES(?,?,?,?,?)")
      .bind(id, actor.account, await digest(invitation), expires, Date.now()).run();
    transportLog("link_created", actor.account, id);
    return Response.json({ id, invitation, expires_at: expires }, { headers: { "cache-control": "no-store" } });
  }
  const match = /^\/v1\/online\/links\/([a-f0-9-]{36})(\/join)?$/.exec(url.pathname);
  if (!match) return new Response("Not found", { status: 404 });
  const link = await db.prepare("SELECT id,owner_id,peer_id FROM online_link WHERE id=?").bind(match[1]).first<Link>();
  if (!link) return new Response("Online link unavailable", { status: 404 });
  if (match[2] && request.method === "POST") {
    const invitation = request.headers.get("x-online-invitation") ?? "";
    if (!/^[a-f0-9]{64}$/.test(invitation) || actor.account === link.owner_id) return new Response("Invalid invitation", { status: 403 });
    const result = await db.prepare("UPDATE online_link SET peer_id=? WHERE id=? AND (peer_id IS NULL OR peer_id=?) AND invitation_hash=? AND expires_at>?")
      .bind(actor.account, link.id, actor.account, await digest(invitation), Date.now()).run();
    if (result.meta.changes !== 1) return new Response("Invitation expired or used", { status: 403 });
    transportLog("link_joined", actor.account, link.id);
    return Response.json({ id: link.id, owner_id: link.owner_id, peer_id: actor.account });
  }
  if (actor.account !== link.owner_id && actor.account !== link.peer_id) return new Response("Online link unavailable", { status: 404 });
  if (request.method === "GET") return Response.json(link);
  if (request.method === "DELETE") {
    await db.prepare("DELETE FROM online_link WHERE id=? AND (owner_id=? OR peer_id=?)").bind(link.id, actor.account, actor.account).run();
    transportLog("link_revoked", actor.account, link.id);
    await Promise.all([link.owner_id, link.peer_id].filter((id): id is string => !!id).map(account => mailbox(env, account).fetch("https://online.internal/revoke", { method: "POST", body: JSON.stringify({ channel: link.id }) })));
    return new Response(null, { status: 204 });
  }
  return new Response("Not found", { status: 404 });
}

/** A bounded durable inbox survives socket loss; ACK follows the receiver's durable write. */
export class OnlineMailbox extends DurableObject<OnlineTransportEnv> {
  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    if (url.pathname === "/deliver" && request.method === "POST") {
      const item = await request.json<Delivery>();
      if (!await this.allowed(item.channel, item.sender, item.recipient)) return new Response("Online link unavailable", { status: 403 });
      const existing = await this.ctx.storage.get<Delivery>(`message:${item.id}`);
      if (existing && (existing.sender !== item.sender || existing.channel !== item.channel)) return new Response("Message identity conflict", { status: 409 });
      if (!existing) {
        const pending = await this.ctx.storage.list<Delivery>({ prefix: "message:" });
        if (pending.size >= MAX_PENDING) return new Response("Recipient inbox is full", { status: 429 });
        await this.ctx.storage.put(`message:${item.id}`, item);
        const alarm = await this.ctx.storage.getAlarm();
        if (alarm === null) await this.ctx.storage.setAlarm(item.created + RETENTION_MS);
      }
      for (const socket of this.ctx.getWebSockets()) {
        if (await this.validSession(socket)) socket.send(JSON.stringify({ kind: "delivery", ...(existing ?? item) }));
      }
      return new Response(null, { status: 204 });
    }
    if (url.pathname === "/revoke" && request.method === "POST") {
      const { channel } = await request.json<{ channel: string }>();
      const pending = await this.ctx.storage.list<Delivery>({ prefix: "message:" });
      for (const [key, item] of pending) if (item.channel === channel) await this.ctx.storage.delete(key);
      for (const socket of this.ctx.getWebSockets()) socket.send(JSON.stringify({ kind: "revoked", channel }));
      return new Response(null, { status: 204 });
    }
    if (request.headers.get("upgrade")?.toLowerCase() !== "websocket") return new Response("Not found", { status: 404 });
    const actor = JSON.parse(request.headers.get("x-online-session") ?? "null") as Session | null;
    if (!actor) return new Response("Unauthorized", { status: 401 });
    const pair = new WebSocketPair();
    for (const previous of this.ctx.getWebSockets()) previous.close(4009, "Online is connected on another device");
    this.ctx.acceptWebSocket(pair[1]);
    pair[1].serializeAttachment(actor);
    transportLog("connected", actor.account);
    pair[1].send(JSON.stringify({ kind: "ready", account_id: actor.account }));
    const pending = await this.ctx.storage.list<Delivery>({ prefix: "message:" });
    for (const [key, item] of pending) {
      if (item.created + RETENTION_MS <= Date.now() || !await this.allowed(item.channel, item.sender, actor.account)) await this.ctx.storage.delete(key);
      else pair[1].send(JSON.stringify({ kind: "delivery", ...item }));
    }
    await this.ctx.storage.setAlarm(Date.now() + 30_000);
    return new Response(null, { status: 101, webSocket: pair[0] });
  }

  private async validSession(socket: WebSocket): Promise<Session | null> {
    const actor = socket.deserializeAttachment() as Session;
    const row = actor.expires > Date.now() && await this.env.ONLINE_DATABASE!.prepare('SELECT id FROM session WHERE id=? AND userId=? AND expiresAt>?').bind(actor.session, actor.account, Date.now()).first();
    if (!row) { transportLog("session_expired", actor.account); socket.close(4001, "Online session expired"); return null; }
    return actor;
  }

  private async allowed(channel: string, sender: string, recipient: string): Promise<boolean> {
    return !!await this.env.ONLINE_DATABASE!.prepare("SELECT id FROM online_link WHERE id=? AND ((owner_id=? AND peer_id=?) OR (owner_id=? AND peer_id=?))")
      .bind(channel, sender, recipient, recipient, sender).first();
  }

  async webSocketMessage(socket: WebSocket, raw: string | ArrayBuffer): Promise<void> {
    const actor = await this.validSession(socket);
    if (!actor) return;
    if (!actor.window || actor.window + 60_000 <= Date.now()) { actor.window = Date.now(); actor.count = 0; }
    actor.count = (actor.count ?? 0) + 1;
    socket.serializeAttachment(actor);
    if (actor.count > 180) { socket.close(1008, "Online message rate exceeded"); return; }
    if (typeof raw !== "string" || new TextEncoder().encode(raw).byteLength > MAX_FRAME) { socket.close(1009, "Frame too large"); return; }
    let frame: Record<string, unknown>;
    try { frame = JSON.parse(raw); } catch { socket.close(1008, "Invalid frame"); return; }
    if (!frame || typeof frame !== "object" || Array.isArray(frame)) { socket.close(1008, "Invalid frame"); return; }
    if (frame.kind === "ping") { socket.send(JSON.stringify({ kind: "pong" })); return; }
    if (frame.kind === "ack" && typeof frame.id === "string" && /^[a-f0-9-]{36}$/.test(frame.id)) {
      await this.ctx.storage.delete(`message:${frame.id}`); return;
    }
    if (frame.kind !== "send" || typeof frame.id !== "string" || !/^[a-f0-9-]{36}$/.test(frame.id)
      || typeof frame.channel !== "string" || typeof frame.recipient !== "string" || !encryptedEnvelope(frame.envelope)) {
      socket.send(JSON.stringify({ kind: "error", message: "Only bounded encrypted messages are accepted" })); return;
    }
    if (!await this.allowed(frame.channel, actor.account, frame.recipient)) {
      transportLog("delivery_denied", actor.account, frame.channel);
      socket.send(JSON.stringify({ kind: "rejected", id: frame.id, reason: "link_unavailable" })); return;
    }
    const response = await mailbox(this.env, frame.recipient).fetch("https://online.internal/deliver", {
      method: "POST", body: JSON.stringify({ id: frame.id, channel: frame.channel, sender: actor.account, recipient: frame.recipient, envelope: frame.envelope, created: Date.now() }),
    });
    socket.send(JSON.stringify({ kind: response.ok ? "accepted" : "rejected", id: frame.id, ...(response.ok ? {} : { reason: "recipient_unavailable" }) }));
  }

  async alarm(): Promise<void> {
    for (const socket of this.ctx.getWebSockets()) await this.validSession(socket);
    const pending = await this.ctx.storage.list<Delivery>({ prefix: "message:" });
    for (const [key, item] of pending) if (item.created + RETENTION_MS <= Date.now()) await this.ctx.storage.delete(key);
    if (this.ctx.getWebSockets().length) await this.ctx.storage.setAlarm(Date.now() + 30_000);
    else {
      const expiries = [...pending.values()].map(item => item.created + RETENTION_MS).filter(time => time > Date.now());
      if (expiries.length) await this.ctx.storage.setAlarm(Math.min(...expiries));
    }
  }
}
