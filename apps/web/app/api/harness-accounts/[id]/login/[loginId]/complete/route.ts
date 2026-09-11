import { NextRequest, NextResponse } from "next/server";
import { requireAuth } from "../../../../../../../lib/api/api-auth";
import { apiBaseUrl } from "../../../../../../../lib/api/choruz-api";

export async function POST(request: NextRequest, context: { params: Promise<{ id: string; loginId: string }> }) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const { id, loginId } = await context.params;
  const companyId = request.nextUrl.searchParams.get("company_id")?.trim();
  if (!companyId) return NextResponse.json({ error: "company_id is required" }, { status: 400 });
  const response = await fetch(`${apiBaseUrl()}/v1/companies/${encodeURIComponent(companyId)}/harness-accounts/${encodeURIComponent(id)}/logins/${encodeURIComponent(loginId)}/complete`, {
    method: "POST", headers: { authorization: `Bearer ${auth.token}` }, cache: "no-store",
  });
  if (response.status === 204) return new NextResponse(null, { status: 204 });
  return NextResponse.json(await response.json().catch(() => ({ error: "Unable to check sign-in" })), { status: response.status });
}
