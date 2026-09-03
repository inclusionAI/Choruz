import { DurableObject } from "cloudflare:workers";

import { verifyGatewayTicket, type GatewayTicketPayload } from "./tickets";
import { validCapability } from "./capability";
import { remoteEntryResponse } from "./remote-entry";
import {
  gatewayControlResponse,
  isEncryptedTransportFrame,
  revokedDeviceIdFromControl,
  revokedDeviceIdsFromControl,
  targetDeviceIdFromSessionFrame,
} from "./control";

export interface Env {
  ROOMS: DurableObjectNamespace<GatewayRoom>;
  RATE_LIMITERS: DurableObjectNamespace<PairingRateLimiter>;
  CAPABILITIES: DurableObjectNamespace<CapabilityStore>;
  GATEWAY_AUTH_SECRET: string;
  /** Origin of a hosted Choruz dashboard whose `/remote` page pairs with hosts
   *  through this gateway; unset means `/` and `/remote` only explain where
   *  that page lives. */
  REMOTE_DASHBOARD_URL?: string;
}

type SocketAttachment = {
  role: GatewayTicketPayload["role"] | "pair_client";
  scope: GatewayTicketPayload["scope"];
  room: string;
  device_id?: string;
  pairing_id?: string;
};

const MAX_FRAME_BYTES = 1_000_000;
const CAPABILITY_PREFIX = "opaque.";

function pairingLog(phase: string, details: Record<string, string | number | boolean | null> = {}): void {
  console.log(JSON.stringify({ event: "remote_control_pairing", phase, ...details }));
}

function pairingDetails(attachment: Pick<SocketAttachment, "pairing_id" | "role" | "scope">): Record<string, string | null> {
  return {
    pairing_id: attachment.pairing_id ?? null,
    role: attachment.role,
    scope: attachment.scope,
  };
}

function capabilityStore(env: Env): DurableObjectStub<CapabilityStore> {
  return env.CAPABILITIES.get(env.CAPABILITIES.idFromName("tickets"));
}

async function readOpaqueTicket(env: Env, ticket: string): Promise<GatewayTicketPayload | null> {
  if (!ticket.startsWith(CAPABILITY_PREFIX)) return null;
  const response = await capabilityStore(env).fetch(`https://capability.internal/read?ticket=${encodeURIComponent(ticket.slice(CAPABILITY_PREFIX.length))}`);
  if (!response.ok) return null;
  return await response.json<GatewayTicketPayload>();
}
export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    if (url.pathname === "/healthz") return Response.json({ ok: true });
    if (url.pathname === "/v1/capabilities" && request.method === "POST") {
      return capabilityStore(env).fetch(request);
    }
    if ((url.pathname === "/" || url.pathname === "/remote") && request.method === "GET") {
      return remoteEntryResponse(url, env.REMOTE_DASHBOARD_URL);
    }
    if (url.pathname !== "/connect" || request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
      return new Response("Not found", { status: 404 });
    }

    const pairingId = url.searchParams.get("pairing_id");
    const isPairClient = url.searchParams.get("role") === "pair_client";
    let attachment: SocketAttachment;

    if (isPairClient) {
      pairingLog("credential_submitted", {
        pairing_id: pairingId,
        identifier_format_valid: typeof pairingId === "string" && /^[A-Za-z0-9_-]{22}$/u.test(pairingId),
      });
    }
    if (isPairClient && pairingId && /^[A-Za-z0-9_-]{22}$/u.test(pairingId)) {
      const clientAddress = request.headers.get("cf-connecting-ip") ?? "local-development";
      const limiter = env.RATE_LIMITERS.get(env.RATE_LIMITERS.idFromName(clientAddress));
      const rateLimit = await limiter.fetch("https://rate-limit.internal/check");
      if (!rateLimit.ok) {
        pairingLog("credential_rejected", { pairing_id: pairingId, reason: "rate_limited", status: rateLimit.status });
        return rateLimit;
      }
      const pairing = await capabilityStore(env).fetch(`https://capability.internal/pair?pairing_id=${pairingId}`);
      if (!pairing.ok) {
        pairingLog("credential_rejected", { pairing_id: pairingId, reason: "invalid_expired_or_not_ready", status: pairing.status });
        return new Response("Pairing credential is invalid, expired, or not ready", { status: 404 });
      }
      const redeemed = await pairing.json<{ room: string; pairing_id?: string }>();
      attachment = { role: "pair_client", scope: "pair", room: redeemed.room, pairing_id: redeemed.pairing_id };
    } else {
      const ticket = url.searchParams.get("ticket");
      const secret = env.GATEWAY_AUTH_SECRET;
      const payload = ticket
        ? await readOpaqueTicket(env, ticket) ?? (
          secret && new TextEncoder().encode(secret).byteLength >= 32
            ? await verifyGatewayTicket(ticket, secret)
            : null
        )
        : null;
      if (!payload) return new Response("Invalid or expired gateway ticket", { status: 401 });
      attachment = payload;
    }

    const roomName = `${attachment.scope}:${attachment.room}`;
    const room = env.ROOMS.get(env.ROOMS.idFromName(roomName));
    const headers = new Headers(request.headers);
    headers.set("x-choruz-attachment", JSON.stringify(attachment));
    const response = await room.fetch(new Request(request, { headers }));
    if (isPairClient) {
      pairingLog(response.status === 101 ? "credential_accepted" : "credential_rejected", {
        ...pairingDetails(attachment),
        status: response.status,
      });
    }
    return response;
  },
};

export class CapabilityStore extends DurableObject<Env> {
  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    if (request.method === "POST" && url.pathname === "/v1/capabilities") {
      const body = await request.json<{ issuer?: string; payload?: GatewayTicketPayload; pairing_id?: string }>().catch(() => null);
      const issuer = body?.issuer;
      const payload = body?.payload;
      if (!issuer || issuer.length < 32 || !payload || !validCapability(payload)) {
        return new Response("Invalid capability request", { status: 400 });
      }
      const issuerKey = `issuer:${payload.scope}:${payload.room}`;
      const existingIssuer = await this.ctx.storage.get<string>(issuerKey);
      if (existingIssuer && existingIssuer !== issuer) {
        return new Response("Capability issuer is not authorized for this room", { status: 403 });
      }
      if (!existingIssuer) {
        await this.ctx.storage.put(issuerKey, issuer);
      }
      if (payload.scope === "pair") {
        if (
          !body?.pairing_id
          || !/^[A-Za-z0-9_-]{22}$/u.test(body.pairing_id)
          || body.pairing_id !== payload.pairing_id
        ) {
          return new Response("Pairing capabilities require their opaque identifier", { status: 400 });
        }
        await this.ctx.storage.put(`pair:${body.pairing_id}`, {
          room: payload.room,
          exp: payload.exp,
          pairing_id: payload.pairing_id,
        });
      }
      const token = crypto.randomUUID().replaceAll("-", "") + crypto.randomUUID().replaceAll("-", "");
      await this.ctx.storage.put(`ticket:${token}`, payload);
      return Response.json({ ticket: `${CAPABILITY_PREFIX}${token}` });
    }
    if (request.method === "GET" && url.pathname === "/read") {
      const ticket = url.searchParams.get("ticket");
      if (!ticket || !/^[a-f0-9]{64}$/iu.test(ticket)) return new Response("Not found", { status: 404 });
      const payload = await this.ctx.storage.get<GatewayTicketPayload>(`ticket:${ticket}`);
      if (!payload || !validCapability(payload)) return new Response("Not found", { status: 404 });
      return Response.json(payload);
    }
    if (request.method === "GET" && url.pathname === "/pair") {
      const pairingId = url.searchParams.get("pairing_id");
      if (!pairingId || !/^[A-Za-z0-9_-]{22}$/u.test(pairingId)) return new Response("Not found", { status: 404 });
      const key = `pair:${pairingId}`;
      const pairing = await this.ctx.storage.get<{ room: string; exp: number; pairing_id?: string }>(key);
      if (!pairing || pairing.exp <= Date.now() / 1_000) {
        if (pairing) await this.ctx.storage.delete(key);
        return new Response("Not found", { status: 404 });
      }
      return Response.json({ room: pairing.room, pairing_id: pairing.pairing_id });
    }
    return new Response("Not found", { status: 404 });
  }
}

export class PairingRateLimiter extends DurableObject<Env> {
  async fetch(): Promise<Response> {
    const now = Date.now();
    const windowMs = 60_000;
    const limit = 20;
    let state = await this.ctx.storage.get<{ started_at: number; count: number }>("window");
    if (!state || now - state.started_at >= windowMs) {
      state = { started_at: now, count: 0 };
    }
    if (state.count >= limit) {
      const retryAfter = Math.max(1, Math.ceil((windowMs - (now - state.started_at)) / 1_000));
      return new Response("Too many pairing attempts", {
        status: 429,
        headers: { "retry-after": String(retryAfter) },
      });
    }
    state.count += 1;
    await this.ctx.storage.put("window", state);
    return new Response(null, { status: 204 });
  }
}

export class GatewayRoom extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
  }

  async fetch(request: Request): Promise<Response> {
    let attachment: SocketAttachment;
    try {
      attachment = JSON.parse(request.headers.get("x-choruz-attachment") ?? "") as SocketAttachment;
    } catch {
      return new Response("Missing connection metadata", { status: 400 });
    }
    const sockets = this.ctx.getWebSockets();
    let deviceRevoked = false;
    if (attachment.role === "device" && attachment.scope === "session" && attachment.device_id) {
      const revoked = await this.ctx.storage.get<string[]>("revoked-device-ids") ?? [];
      deviceRevoked = revoked.includes(attachment.device_id);
    }
    if (
      attachment.role === "pair_client" &&
      !sockets.some((socket) => (socket.deserializeAttachment() as SocketAttachment | null)?.role === "host")
    ) {
      pairingLog("socket_rejected", { ...pairingDetails(attachment), reason: "host_not_connected" });
      return new Response("Pairing credential is invalid, expired, or not ready", { status: 404 });
    }
    if (attachment.role === "pair_client") {
      if (!attachment.pairing_id) {
        pairingLog("socket_rejected", { ...pairingDetails(attachment), reason: "missing_pairing_id" });
        return new Response("Pairing credential is invalid, expired, or not ready", { status: 404 });
      }
      const consumedKey = `consumed-pairing:${attachment.pairing_id}`;
      if (await this.ctx.storage.get<boolean>(consumedKey)) {
        pairingLog("socket_rejected", { ...pairingDetails(attachment), reason: "already_used" });
        return new Response("Pairing credential is invalid, expired, or not ready", { status: 404 });
      }
      await this.ctx.storage.put(consumedKey, true);
    }
    for (const socket of sockets) {
      const existing = socket.deserializeAttachment() as SocketAttachment | null;
      if (
        existing?.role === attachment.role &&
        attachment.role !== "device"
      ) socket.close(4001, "Replaced by a newer connection");
    }
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    this.ctx.acceptWebSocket(server);
    server.serializeAttachment(attachment);
    if (attachment.scope === "pair") pairingLog("socket_connected", pairingDetails(attachment));
    if (deviceRevoked) {
      server.close(4003, "Device revoked");
      return new Response(null, { status: 101, webSocket: client });
    }
    server.send(JSON.stringify({ type: "gateway.ready", transport: "cloud" }));
    if (attachment.scope === "session") {
      const joined = JSON.stringify({
        type: "gateway.peer_joined",
        role: attachment.role,
        device_id: attachment.device_id,
      });
      for (const peer of sockets) {
        if (peer.readyState !== WebSocket.OPEN) continue;
        const existing = peer.deserializeAttachment() as SocketAttachment | null;
        if (existing?.role !== attachment.role) peer.send(joined);
      }
    }
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(socket: WebSocket, message: string | ArrayBuffer): Promise<void> {
    const sender = socket.deserializeAttachment() as SocketAttachment;
    const size = typeof message === "string" ? new TextEncoder().encode(message).byteLength : message.byteLength;
    if (size > MAX_FRAME_BYTES) {
      if (sender.scope === "pair") pairingLog("socket_rejected", { ...pairingDetails(sender), reason: "frame_too_large" });
      socket.close(1009, "Frame too large");
      return;
    }

    if (sender.scope === "pair" && typeof message === "string") {
      try {
        const value = JSON.parse(message) as { kind?: unknown };
        if (value.kind === "pair.commit" || value.kind === "pair.reveal" || value.kind === "pair.proof" || value.kind === "pair.complete") {
          pairingLog("protocol_message", { ...pairingDetails(sender), kind: value.kind });
        }
      } catch {
        pairingLog("protocol_message_rejected", { ...pairingDetails(sender), reason: "invalid_json" });
      }
    }

    // Latency probes are gateway control frames, not user or agent data. Echo
    // them locally so a host that moved networks can detect that its current
    // room is now far away and create a fresh, locally placed transport room.
    if (typeof message === "string") {
      const controlResponse = gatewayControlResponse(message);
      if (controlResponse) {
        socket.send(controlResponse);
        return;
      }
      if (sender.role === "host" && sender.scope === "session") {
        const revoked = revokedDeviceIdsFromControl(message);
        if (revoked) {
          await this.ctx.storage.put("revoked-device-ids", revoked);
          for (const peer of this.ctx.getWebSockets()) {
            const attachment = peer.deserializeAttachment() as SocketAttachment;
            if (attachment.role === "device" && attachment.device_id && revoked.includes(attachment.device_id)) {
              peer.close(4003, "Device revoked");
            }
          }
          return;
        }
        const revokedDeviceId = revokedDeviceIdFromControl(message);
        if (revokedDeviceId) {
          const existing = await this.ctx.storage.get<string[]>("revoked-device-ids") ?? [];
          const next = existing.includes(revokedDeviceId)
            ? existing
            : [...existing, revokedDeviceId];
          await this.ctx.storage.put("revoked-device-ids", next);
          for (const peer of this.ctx.getWebSockets()) {
            const attachment = peer.deserializeAttachment() as SocketAttachment;
            if (attachment.role === "device" && attachment.device_id === revokedDeviceId) {
              peer.close(4003, "Device revoked");
            }
          }
          return;
        }
      }
    }
    if (sender.scope === "transport") {
      if (typeof message !== "string") {
        socket.close(1003, "Encrypted JSON frames required");
        return;
      }
      if (!isEncryptedTransportFrame(message)) {
        socket.close(1008, "End-to-end encryption required");
        return;
      }
    }
    for (const peer of this.ctx.getWebSockets()) {
      if (peer === socket || peer.readyState !== WebSocket.OPEN) continue;
      const recipient = peer.deserializeAttachment() as SocketAttachment;
      if (recipient.role === sender.role) continue;
      if (sender.scope === "session" && typeof message === "string") {
        const targetDeviceId = targetDeviceIdFromSessionFrame(message);
        if (targetDeviceId && recipient.device_id !== targetDeviceId) continue;
      }
      peer.send(message);
    }
  }

  async webSocketClose(socket: WebSocket, code: number, reason: string): Promise<void> {
    const sender = socket.deserializeAttachment() as SocketAttachment | null;
    if (sender?.scope === "pair") {
      pairingLog("socket_closed", {
        ...pairingDetails(sender),
        close_code: code,
        close_reason_present: reason.length > 0,
      });
    }
    if (sender?.scope === "transport" && sender.role === "host") {
      for (const peer of this.ctx.getWebSockets()) {
        if (peer !== socket) peer.close(4004, "Host transport rotated");
      }
    }
    socket.close(code, reason);
  }
}
