import { describe, expect, it } from "vitest";

import {
  gatewayControlResponse,
  isEncryptedTransportFrame,
  revokedDeviceIdsFromControl,
  revokedDeviceIdFromControl,
  targetDeviceIdFromSessionFrame,
} from "./control";

describe("gatewayControlResponse", () => {
  it("echoes a bounded latency probe without forwarding it as agent data", () => {
    expect(gatewayControlResponse(JSON.stringify({ type: "gateway.ping", nonce: "probe-1" })))
      .toBe(JSON.stringify({ type: "gateway.pong", nonce: "probe-1" }));
  });

  it("leaves application, malformed, and oversized frames untouched", () => {
    expect(gatewayControlResponse(JSON.stringify({ kind: "message.send" }))).toBeNull();
    expect(gatewayControlResponse("not-json")).toBeNull();
    expect(gatewayControlResponse(JSON.stringify({ type: "gateway.ping", nonce: "x".repeat(129) })))
      .toBeNull();
  });
});

describe("isEncryptedTransportFrame", () => {
  it("accepts only opaque end-to-end encrypted application frames", () => {
    expect(isEncryptedTransportFrame(JSON.stringify({ kind: "e2e", iv: "iv", ciphertext: "cipher" })))
      .toBe(true);
    expect(isEncryptedTransportFrame(JSON.stringify({ kind: "message", payload: { content: "plain" } })))
      .toBe(false);
    expect(isEncryptedTransportFrame(JSON.stringify({ kind: "e2e", iv: "", ciphertext: "cipher" })))
      .toBe(false);
    expect(isEncryptedTransportFrame("not-json")).toBe(false);
  });
});

describe("revokedDeviceIdsFromControl", () => {
  it("accepts a bounded host revocation snapshot", () => {
    expect(revokedDeviceIdsFromControl(JSON.stringify({
      type: "gateway.sync_revocations",
      device_ids: ["device-1", "device-2"],
    }))).toEqual(["device-1", "device-2"]);
  });

  it("rejects malformed revocation snapshots", () => {
    expect(revokedDeviceIdsFromControl(JSON.stringify({
      type: "gateway.sync_revocations",
      device_ids: [""],
    }))).toBeNull();
    expect(revokedDeviceIdsFromControl(JSON.stringify({ type: "other", device_ids: [] })))
      .toBeNull();
  });
});

describe("revokedDeviceIdFromControl", () => {
  it("accepts only a bounded single-device revocation", () => {
    expect(revokedDeviceIdFromControl(JSON.stringify({
      type: "gateway.revoke_device",
      device_id: "device-1",
    }))).toBe("device-1");
    expect(revokedDeviceIdFromControl(JSON.stringify({
      type: "gateway.revoke_device",
      device_id: "",
    }))).toBeNull();
  });
});

describe("targetDeviceIdFromSessionFrame", () => {
  it("extracts only a bounded target device", () => {
    expect(targetDeviceIdFromSessionFrame(JSON.stringify({ target_device_id: "device-1" })))
      .toBe("device-1");
    expect(targetDeviceIdFromSessionFrame(JSON.stringify({ target_device_id: "" }))).toBeNull();
  });
});
