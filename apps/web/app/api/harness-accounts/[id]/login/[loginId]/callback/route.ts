import { NextRequest, NextResponse } from "next/server";

import { requireAuth } from "../../../../../../../lib/api/api-auth";
import { canAccessHarnessAccountCompany } from "../../../../../../../lib/agents/harness-account-access";
import { getHarnessAccount } from "../../../../../../../lib/agents/harness-accounts";
import { apiBaseUrl } from "../../../../../../../lib/api/choruz-api";

export async function POST(request: NextRequest, context: { params: Promise<{ id: string; loginId: string }> }) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const { id, loginId } = await context.params;
  const companyId = request.nextUrl.searchParams.get("company_id")?.trim();
  const body = await request.json().catch(() => null) as { code?: unknown } | null;
  if (!companyId || typeof body?.code !== "string") return NextResponse.json({ error: "company_id and code are required" }, { status: 400 });
  if (!(await canAccessHarnessAccountCompany(auth.token, companyId))) return NextResponse.json({ error: "Forbidden" }, { status: 403 });
  const account = await getHarnessAccount(id, companyId);
  if (!account) return NextResponse.json({ error: "Harness account not found" }, { status: 404 });
  const response = await fetch(
    `${apiBaseUrl()}/v1/companies/${encodeURIComponent(companyId)}/harness-accounts/${encodeURIComponent(account.id)}/logins/${encodeURIComponent(loginId)}/callback`,
    {
      method: "POST",
      headers: { authorization: `Bearer ${auth.token}`, "content-type": "application/json" },
      body: JSON.stringify({ code: body.code }),
      cache: "no-store",
    },
  );
  if (response.status === 204) return new NextResponse(null, { status: 204 });
  const error = await response.json().catch(() => ({ error: "Unable to submit the authorization code" }));
  return NextResponse.json(error, { status: response.status });
}
