import { NextRequest, NextResponse } from "next/server";

import { requireAuth } from "../../../../../lib/api/api-auth";
import { canAccessHarnessAccountCompany } from "../../../../../lib/agents/harness-account-access";
import { getHarnessAccount, probeHarnessAccount } from "../../../../../lib/agents/harness-accounts";

export async function POST(request: NextRequest, context: { params: Promise<{ id: string }> }) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const { id } = await context.params;
  const companyId = request.nextUrl.searchParams.get("company_id")?.trim();
  if (!companyId) return NextResponse.json({ error: "company_id is required" }, { status: 400 });
  if (!(await canAccessHarnessAccountCompany(auth.token, companyId))) return NextResponse.json({ error: "Forbidden" }, { status: 403 });
  const account = await getHarnessAccount(id, companyId);
  if (!account) return NextResponse.json({ error: "Harness account not found" }, { status: 404 });
  try {
    return NextResponse.json(await probeHarnessAccount(account));
  } catch (error) {
    return NextResponse.json({ error: error instanceof Error ? error.message : "Account probe failed" }, { status: 409 });
  }
}
