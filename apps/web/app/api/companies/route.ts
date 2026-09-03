import { NextRequest, NextResponse } from "next/server";
import { apiBaseUrl } from "../../../lib/api/choruz-api";
import { requireAuth } from "../../../lib/api/api-auth";

export async function GET(request: NextRequest) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const { token } = auth;

  try {
    const resp = await fetch(`${apiBaseUrl()}/v1/companies`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    const data = await resp.json();
    return NextResponse.json(data, { status: resp.status });
  } catch (err) {
    return NextResponse.json(
      { error: err instanceof Error ? err.message : "Internal server error" },
      { status: 500 },
    );
  }
}

export async function POST(request: NextRequest) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const { token } = auth;

  try {
    const body = await request.json();
    const resp = await fetch(`${apiBaseUrl()}/v1/companies`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify(body),
    });
    const data = await resp.json();
    return NextResponse.json(data, { status: resp.status });
  } catch (err) {
    return NextResponse.json(
      { error: err instanceof Error ? err.message : "Internal server error" },
      { status: 500 },
    );
  }
}
