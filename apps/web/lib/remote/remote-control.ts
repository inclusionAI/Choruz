export type RemoteControlSettings = {
  gateway_url: string | null;
  gateway_ticket: string | null;
};

export type RemoteControlPairing = {
  pairing_id: string;
  credential: string;
  expires_at: string;
  gateway_url: string;
};

export type RemoteControlDevice = {
  id: string;
  name: string;
  paired_at: string;
  last_seen_at: string | null;
};

export type RuntimeHost = {
  id: string;
  company_id: string;
  name: string;
  status: "online" | "offline" | "revoked";
  last_seen_at: string | null;
  created_at: string;
};

export type RuntimeHostPairing = {
  code: string;
  expires_at: string;
};

/** The dashboard's own `/remote` page, prefilled so it pairs at once. */
export function remoteDashboardPath(
  gatewayUrl: string,
  credential: string,
  deviceName: string,
): string {
  const normalizedCredential = parsePairingCredential(credential).value;
  if (!gatewayUrl.trim()) throw new Error("Cloud Gateway is not configured.");
  const params = new URLSearchParams({
    gateway: gatewayUrl.trim(),
    device_name: deviceName.trim() || "Choruz browser",
  });
  return `/remote?${params}#credential=${encodeURIComponent(normalizedCredential)}`;
}

export type PairingCredential = {
  value: string;
  id: string;
  secret: string;
};

const PAIRING_CREDENTIAL_PATTERN = /^v1\.([A-Za-z0-9_-]{22})\.([A-Za-z0-9_-]{22})$/u;

export function parsePairingCredential(input: string): PairingCredential {
  const value = input.trim();
  const match = PAIRING_CREDENTIAL_PATTERN.exec(value);
  if (!match) throw new Error("Paste the complete pairing credential.");
  return { value, id: match[1], secret: match[2] };
}
