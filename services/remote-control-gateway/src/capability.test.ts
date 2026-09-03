import { describe, expect, it } from "vitest";

import { validCapability } from "./capability";

describe("opaque gateway capabilities", () => {
  const active = {
    room: "019d1234-5678-7abc-8def-0123456789ab",
    role: "host" as const,
    scope: "pair" as const,
    exp: Math.floor(Date.now() / 1_000) + 60,
  };

  it("accepts only a live, well-formed stored payload", () => {
    expect(validCapability(active)).toBe(true);
    expect(validCapability({ ...active, exp: Math.floor(Date.now() / 1_000) - 1 })).toBe(false);
    expect(validCapability({ ...active, role: "pair_client" as never })).toBe(false);
    expect(validCapability({ ...active, room: "12345678" })).toBe(false);
    expect(validCapability({ ...active, room: "a".repeat(32) })).toBe(false);
    expect(validCapability({ ...active, room: 42 as never })).toBe(false);
    expect(validCapability({ ...active, pairing_id: "pairing-1" })).toBe(true);
    expect(validCapability({ ...active, pairing_id: "" })).toBe(false);
    expect(validCapability({ ...active, pairing_id: 42 as never })).toBe(false);
  });

  it("requires a device identity for a rendezvous capability", () => {
    expect(validCapability({ ...active, role: "device", scope: "session" })).toBe(false);
    expect(validCapability({ ...active, role: "device", scope: "session", device_id: 1 as never })).toBe(false);
    expect(validCapability({ ...active, role: "device", scope: "session", device_id: "device-1" })).toBe(true);
  });
});
