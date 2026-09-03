import { describe, expect, it } from "vitest";

import { verifyGatewayTicket, type GatewayTicketPayload } from "./tickets";

function base64Url(value: Uint8Array): string {
  return Buffer.from(value).toString("base64url");
}

async function createTicket(payload: GatewayTicketPayload, secret: string): Promise<string> {
  const encoded = base64Url(new TextEncoder().encode(JSON.stringify(payload)));
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const signature = await crypto.subtle.sign(
    "HMAC",
    key,
    new TextEncoder().encode(`gateway-ticket\0${encoded}`),
  );
  return `${encoded}.${Buffer.from(signature).toString("hex")}`;
}

describe("verifyGatewayTicket", () => {
  const secret = "test-secret-with-enough-entropy";
  const payload: GatewayTicketPayload = {
    room: "12345678",
    role: "host",
    scope: "pair",
    exp: 2_000,
  };

  it("accepts a correctly signed, unexpired ticket", async () => {
    await expect(verifyGatewayTicket(await createTicket(payload, secret), secret, 1_000))
      .resolves.toEqual(payload);
  });

  it("preserves an opaque pairing diagnostic id without accepting an empty one", async () => {
    const traced = { ...payload, pairing_id: "019d1234-5678-7abc-8def-0123456789ab" };
    await expect(verifyGatewayTicket(await createTicket(traced, secret), secret, 1_000))
      .resolves.toEqual(traced);
    const empty = { ...payload, pairing_id: "" };
    await expect(verifyGatewayTicket(await createTicket(empty, secret), secret, 1_000))
      .resolves.toBeNull();
  });

  it("rejects tampered and expired tickets", async () => {
    const ticket = await createTicket(payload, secret);
    const tampered = `${ticket.slice(0, -1)}${ticket.endsWith("0") ? "1" : "0"}`;
    await expect(verifyGatewayTicket(tampered, secret, 1_000)).resolves.toBeNull();
    await expect(verifyGatewayTicket(ticket, secret, 2_000)).resolves.toBeNull();
  });

  it("rejects missing and non-finite expirations", async () => {
    const missing = { ...payload, exp: undefined } as unknown as GatewayTicketPayload;
    await expect(verifyGatewayTicket(await createTicket(missing, secret), secret, 1_000))
      .resolves.toBeNull();
    const infinite = { ...payload, exp: Number.POSITIVE_INFINITY };
    await expect(verifyGatewayTicket(await createTicket(infinite, secret), secret, 1_000))
      .resolves.toBeNull();
  });

  it("accepts ephemeral transport-room tickets", async () => {
    const transport = { ...payload, room: "session-unique-id", scope: "transport" as const };
    await expect(verifyGatewayTicket(await createTicket(transport, secret), secret, 1_000))
      .resolves.toEqual(transport);
  });

  it("requires a device identity on rendezvous tickets", async () => {
    const missingIdentity = { ...payload, role: "device" as const, scope: "session" as const };
    await expect(verifyGatewayTicket(await createTicket(missingIdentity, secret), secret, 1_000))
      .resolves.toBeNull();
    const identified = { ...missingIdentity, device_id: "device-1" };
    await expect(verifyGatewayTicket(await createTicket(identified, secret), secret, 1_000))
      .resolves.toEqual(identified);
  });
});
