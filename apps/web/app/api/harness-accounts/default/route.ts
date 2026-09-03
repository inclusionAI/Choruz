import { NextRequest, NextResponse } from "next/server";

import { requireAuth } from "../../../../lib/api/api-auth";
import { canAccessHarnessAccountCompany } from "../../../../lib/agents/harness-account-access";
import {
  defaultHarnessAccountWasRemoved,
  ensureDefaultHarnessAccount,
  getHarnessAccount,
  probeHarnessAccount,
  runtimeHostBelongsToCompany,
  type AccountDriver,
} from "../../../../lib/agents/harness-accounts";

const DRIVERS = new Set<AccountDriver>(["claude_terminal", "codex_terminal"]);

/**
 * Register the login a device already has as its default account. With
 * `probe: true` a local account that has not verified yet is verified now.
 * A removed default stays hidden unless `reactivate_removed` is true; that
 * case returns 204 so opening the account manager does not recreate it. A
 * failed probe answers with the account and its recorded status rather
 * than an error, so the caller can show it.
 */
export async function POST(request: NextRequest) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const body = await request.json().catch(() => null) as Record<string, unknown> | null;
  const companyId = typeof body?.company_id === "string" ? body.company_id.trim() : "";
  const runtimeHostId = typeof body?.runtime_host_id === "string" ? body.runtime_host_id.trim() || null : null;
  const driverType = body?.driver_type as AccountDriver;
  if (!companyId || !DRIVERS.has(driverType)) {
    return NextResponse.json({ error: "A valid company and driver are required" }, { status: 400 });
  }
  if (!(await canAccessHarnessAccountCompany(auth.token, companyId))) return NextResponse.json({ error: "Forbidden" }, { status: 403 });
  if (runtimeHostId && !(await runtimeHostBelongsToCompany(runtimeHostId, companyId))) {
    return NextResponse.json({ error: "The selected runtime host does not belong to this company" }, { status: 404 });
  }
  const defaultAccount = { companyId, runtimeHostId, driverType };
  if (body?.reactivate_removed !== true && await defaultHarnessAccountWasRemoved(defaultAccount)) {
    return new NextResponse(null, { status: 204 });
  }
  const account = await ensureDefaultHarnessAccount(defaultAccount);
  if (body?.probe !== true || account.runtimeHostId || account.status === "active") {
    return NextResponse.json(account);
  }
  try {
    return NextResponse.json(await probeHarnessAccount(account));
  } catch {
    return NextResponse.json((await getHarnessAccount(account.id, companyId)) ?? account);
  }
}
