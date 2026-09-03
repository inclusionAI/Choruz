export function gatewayControlResponse(message: string): string | null {
  try {
    const control = JSON.parse(message) as { type?: unknown; nonce?: unknown };
    if (control.type !== "gateway.ping" || typeof control.nonce !== "string") return null;
    if (control.nonce.length === 0 || control.nonce.length > 128) return null;
    return JSON.stringify({ type: "gateway.pong", nonce: control.nonce });
  } catch {
    return null;
  }
}

export function isEncryptedTransportFrame(message: string): boolean {
  try {
    const frame = JSON.parse(message) as { kind?: unknown; iv?: unknown; ciphertext?: unknown };
    return frame.kind === "e2e"
      && typeof frame.iv === "string"
      && frame.iv.length > 0
      && typeof frame.ciphertext === "string"
      && frame.ciphertext.length > 0;
  } catch {
    return false;
  }
}

export function revokedDeviceIdsFromControl(message: string): string[] | null {
  try {
    const frame = JSON.parse(message) as { type?: unknown; device_ids?: unknown };
    if (frame.type !== "gateway.sync_revocations" || !Array.isArray(frame.device_ids)) return null;
    if (frame.device_ids.length > 10_000) return null;
    const ids = frame.device_ids.filter(
      (id): id is string => typeof id === "string" && id.length > 0 && id.length <= 128,
    );
    return ids.length === frame.device_ids.length ? ids : null;
  } catch {
    return null;
  }
}

export function revokedDeviceIdFromControl(message: string): string | null {
  try {
    const frame = JSON.parse(message) as { type?: unknown; device_id?: unknown };
    return frame.type === "gateway.revoke_device"
      && typeof frame.device_id === "string"
      && frame.device_id.length > 0
      && frame.device_id.length <= 128
      ? frame.device_id
      : null;
  } catch {
    return null;
  }
}

export function targetDeviceIdFromSessionFrame(message: string): string | null {
  try {
    const frame = JSON.parse(message) as { target_device_id?: unknown };
    return typeof frame.target_device_id === "string"
      && frame.target_device_id.length > 0
      && frame.target_device_id.length <= 128
      ? frame.target_device_id
      : null;
  } catch {
    return null;
  }
}
