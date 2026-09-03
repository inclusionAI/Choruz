import { NextRequest, NextResponse } from "next/server";
import { apiBaseUrl } from "../../../../lib/api/choruz-api";
import { requireAuth } from "../../../../lib/api/api-auth";

export async function POST(request: NextRequest) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const { token } = auth;

  try {
    const body = await request.json();

    const res = await fetch(`${apiBaseUrl()}/v1/agents/batch-disable`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify(body),
    });

    const data = await res.json();
    return NextResponse.json(data, { status: res.status });
  } catch (err) {
    return NextResponse.json(
      { error: err instanceof Error ? err.message : "Internal server error" },
      { status: 500 },
    );
  }
}
