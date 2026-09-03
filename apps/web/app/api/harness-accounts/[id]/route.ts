import { NextRequest, NextResponse } from "next/server";

import { requireAuth } from "../../../../lib/api/api-auth";
import { canAccessHarnessAccountCompany } from "../../../../lib/agents/harness-account-access";
import { disableHarnessAccount, getHarnessAccount } from "../../../../lib/agents/harness-accounts";

export async function GET(request: NextRequest, context: { params: Promise<{ id: string }> }) {
  const resolved = await resolveAccountRequest(request, context);
  if (resolved instanceof NextResponse) return resolved;
  return NextResponse.json(resolved.account);
}

export async function DELETE(request: NextRequest, context: { params: Promise<{ id: string }> }) {
  const resolved = await resolveAccountRequest(request, context);
  if (resolved instanceof NextResponse) return resolved;
  try {
    const disabledBindings = await disableHarnessAccount(resolved.account.id, resolved.companyId);
    return NextResponse.json({ disabled_bindings: disabledBindings });
  } catch (error) {
    const message = error instanceof Error ? error.message : "Unable to remove harness account";
    const status = /not found/i.test(message) ? 404 : 500;
    return NextResponse.json({ error: message }, { status });
  }
}

async function resolveAccountRequest(request: NextRequest, context: { params: Promise<{ id: string }> }) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const { id } = await context.params;
  const companyId = request.nextUrl.searchParams.get("company_id")?.trim();
  if (!companyId) return NextResponse.json({ error: "company_id is required" }, { status: 400 });
  if (!(await canAccessHarnessAccountCompany(auth.token, companyId))) return NextResponse.json({ error: "Forbidden" }, { status: 403 });
  const account = await getHarnessAccount(id, companyId);
  if (!account) return NextResponse.json({ error: "Harness account not found" }, { status: 404 });
  return { account, companyId };
}
