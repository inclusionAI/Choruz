import { apiBaseUrl } from "../api/choruz-api";

/** Keep device authorization and dispatch in the gateway's runtime-host owner. */
export function deviceDriverCatalog(token: string, hostId: string | null, driverType?: string): Promise<Response> {
  if (!hostId) return fetch(`${apiBaseUrl()}/v1/drivers/models?driver_type=${encodeURIComponent(driverType ?? "")}`, {
    headers: { Authorization: `Bearer ${token}` }, cache: "no-store",
  });
  return fetch(`${apiBaseUrl()}/v1/runtime-hosts/${encodeURIComponent(hostId)}/operations`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ kind: "drivers.inspect", request: driverType ? { driver_type: driverType } : {} }),
    cache: "no-store",
  });
}
