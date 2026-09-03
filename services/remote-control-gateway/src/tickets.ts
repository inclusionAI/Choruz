export type GatewayTicketPayload = {
  room: string;
  role: "host" | "device";
  scope: "pair" | "session" | "transport";
  exp: number;
  device_id?: string;
  pairing_id?: string;
};

function base64UrlDecode(value: string): Uint8Array {
  const padded = value.replace(/-/g, "+").replace(/_/g, "/") + "===".slice((value.length + 3) % 4);
  return Uint8Array.from(atob(padded), (character) => character.charCodeAt(0));
}

function hexDecode(value: string): Uint8Array | null {
  if (!/^[a-f0-9]{64}$/iu.test(value)) return null;
  return Uint8Array.from(value.match(/.{2}/gu) ?? [], (pair) => Number.parseInt(pair, 16));
}

export async function verifyGatewayTicket(
  ticket: string,
  secret: string,
  nowSeconds = Date.now() / 1000,
): Promise<GatewayTicketPayload | null> {
  const [encoded, signature, extra] = ticket.split(".");
  if (!encoded || !signature || extra) return null;
  const provided = hexDecode(signature);
  if (!provided) return null;
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["verify"],
  );
  const valid = await crypto.subtle.verify(
    "HMAC",
    key,
    provided,
    new TextEncoder().encode(`gateway-ticket\0${encoded}`),
  );
  if (!valid) return null;
  try {
    const payload = JSON.parse(new TextDecoder().decode(base64UrlDecode(encoded))) as GatewayTicketPayload;
    if (
      !payload.room ||
      !["host", "device"].includes(payload.role) ||
      !["pair", "session", "transport"].includes(payload.scope) ||
      (payload.device_id !== undefined && (typeof payload.device_id !== "string" || !payload.device_id)) ||
      (payload.pairing_id !== undefined && (typeof payload.pairing_id !== "string" || !payload.pairing_id)) ||
      (payload.role === "device" && payload.scope === "session" && !payload.device_id) ||
      typeof payload.exp !== "number" ||
      !Number.isFinite(payload.exp) ||
      payload.exp <= nowSeconds
    ) return null;
    return payload;
  } catch {
    return null;
  }
}
