import { NextRequest, NextResponse } from "next/server";

import { requireAuth } from "../../../lib/api/api-auth";
import { canAccessHarnessAccountCompany } from "../../../lib/agents/harness-account-access";
import {
  createHarnessAccount,
  listHarnessAccounts,
  runtimeHostBelongsToCompany,
  type AccountDriver,
  type HarnessAccountProfileKind,
} from "../../../lib/agents/harness-accounts";

const DRIVERS = new Set<AccountDriver>(["claude_terminal", "codex_terminal"]);
const PROFILE_KINDS = new Set<HarnessAccountProfileKind>(["default", "isolated"]);

export async function GET(request: NextRequest) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const companyId = request.nextUrl.searchParams.get("company_id")?.trim();
  const runtimeHostId = request.nextUrl.searchParams.get("runtime_host_id")?.trim() || null;
  if (!companyId) return NextResponse.json({ error: "company_id is required" }, { status: 400 });
  if (!(await canAccessHarnessAccountCompany(auth.token, companyId))) return NextResponse.json({ error: "Forbidden" }, { status: 403 });
  return NextResponse.json({ accounts: await listHarnessAccounts(companyId, runtimeHostId) });
}

export async function POST(request: NextRequest) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const body = await request.json().catch(() => null) as Record<string, unknown> | null;
  const companyId = typeof body?.company_id === "string" ? body.company_id.trim() : "";
  const runtimeHostId = typeof body?.runtime_host_id === "string" ? body.runtime_host_id.trim() || null : null;
  const driverType = body?.driver_type as AccountDriver;
  const profileKind = body?.profile_kind as HarnessAccountProfileKind;
  const name = typeof body?.name === "string" ? body.name.trim() : "";
  if (!companyId || !name || name.length > 80 || !DRIVERS.has(driverType) || !PROFILE_KINDS.has(profileKind)) {
    return NextResponse.json({ error: "A valid company, name, driver and profile kind are required" }, { status: 400 });
  }
  if (!(await canAccessHarnessAccountCompany(auth.token, companyId))) return NextResponse.json({ error: "Forbidden" }, { status: 403 });
  if (runtimeHostId && !(await runtimeHostBelongsToCompany(runtimeHostId, companyId))) {
    return NextResponse.json({ error: "The selected runtime host does not belong to this company" }, { status: 404 });
  }
  try {
    return NextResponse.json(await createHarnessAccount({ companyId, runtimeHostId, driverType, name, profileKind }), { status: 201 });
  } catch (error) {
    if (error instanceof Error && /unique/i.test(error.message)) {
      return NextResponse.json({ error: "An account with that name or default profile already exists" }, { status: 409 });
    }
    return NextResponse.json({ error: "Unable to create harness account" }, { status: 500 });
  }
}
