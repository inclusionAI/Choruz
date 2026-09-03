import { NextRequest, NextResponse } from "next/server";
import { apiBaseUrl, decodeSessionClaims, sessionCookieName } from "./choruz-api";

export type AuthResult = { token: string; claims: NonNullable<ReturnType<typeof decodeSessionClaims>> };

/**
 * Validate the session cookie by round-tripping to the gateway, which
 * verifies the HMAC signature and expiry. Returns token + claims on success,
 * or a 401 NextResponse on failure.
 *
 * Do NOT rely on `decodeSessionClaims` alone — that function only parses
 * the unsigned payload and trusts whatever the caller put there. Any Next.js
 * route that acts on the returned principal_id / workspace_id / principal_type
 * MUST go through `requireAuth` so the server-side check runs.
 */
export async function requireAuth(
  request: NextRequest,
): Promise<AuthResult | NextResponse> {
  const token = request.cookies.get(sessionCookieName())?.value;
  if (!token) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  // Ask the gateway to verify signature + expiry. Without this the payload
  // can be base64-forged and any downstream route that trusts
  // `claims.principal_id` / `claims.principal_type` is bypassable.
  //
  // Short timeout so a hung gateway can't stall every authenticated request
  // indefinitely; 5xx is reported as 503 (upstream issue) rather than 401
  // (client "unauthenticated"), so a monitor / client retry can do the
  // right thing.
  let verifyRes: Response;
  try {
    verifyRes = await fetch(`${apiBaseUrl()}/v1/me`, {
      headers: { authorization: `Bearer ${token}` },
      cache: "no-store",
      signal: AbortSignal.timeout(3_000),
    });
  } catch {
    return NextResponse.json({ error: "Auth service unavailable" }, { status: 503 });
  }
  if (verifyRes.status === 401 || verifyRes.status === 403) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }
  if (!verifyRes.ok) {
    return NextResponse.json({ error: "Auth service unavailable" }, { status: 503 });
  }

  // Safe to parse the payload for convenience fields — signature + expiry
  // have already been verified by the gateway above.
  const claims = decodeSessionClaims(token);
  if (!claims) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }
  return { token, claims };
}
