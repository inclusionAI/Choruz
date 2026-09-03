import type { GatewayTicketPayload } from "./tickets";

const UUID_V7_ROOM = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu;
const SESSION_ROOM = /^[0-9a-f]{64}$/iu;

function isServerGeneratedRoom(room: unknown): room is string {
  return typeof room === "string" && (UUID_V7_ROOM.test(room) || SESSION_ROOM.test(room));
}

export function validCapability(payload: GatewayTicketPayload): boolean {
  return Boolean(
    // The Worker has no per-installation secret in zero-config hosted mode.
    // Possession of this unguessable room is the capability that authorizes
    // its first issuer binding; later issuers must match the stored value.
    isServerGeneratedRoom(payload.room)
    && ["host", "device"].includes(payload.role)
    && ["pair", "session", "transport"].includes(payload.scope)
    && Number.isFinite(payload.exp) && payload.exp > Date.now() / 1_000
    && (
      payload.role !== "device"
      || payload.scope !== "session"
      || (typeof payload.device_id === "string" && payload.device_id.length > 0)
    )
    && (payload.pairing_id === undefined || (typeof payload.pairing_id === "string" && payload.pairing_id.length > 0)),
  );
}
