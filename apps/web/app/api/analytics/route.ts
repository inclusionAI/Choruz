import { NextRequest, NextResponse } from "next/server";
import { requireAuth } from "../../../lib/api/api-auth";
import { sanitizeTelemetryValue } from "../../../lib/api/telemetry-sanitize";

export async function POST(request: NextRequest) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;

  try {
    const body = await request.json();
    // Legacy analytics is log-only; redact before writing to server logs.
    console.log("[analytics]", JSON.stringify(sanitizeTelemetryValue(body)));
    return NextResponse.json({ ok: true });
  } catch (err) {
    return NextResponse.json(
      { error: err instanceof Error ? err.message : "Internal server error" },
      { status: 500 },
    );
  }
}
