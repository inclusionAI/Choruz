import { NextRequest, NextResponse } from "next/server";

import { requireAuth } from "../../../lib/api/api-auth";
import { hasGroupProvisioningCompanyAccess } from "../../../lib/groups/group-provisioning-company-access";
import {
  createGroupProvisioningRunner,
  defaultGroupProvisioningStore,
} from "../../../lib/groups/group-provisioning-runner";
import { getDriverAvailability } from "../../../lib/drivers/driver-availability";
import type { ProvisioningJobCreationRequest } from "../../../lib/groups/group-provisioning-contract";
import { serverPluginEnabled } from "../../../plugins/server-plugin";

export async function POST(request: NextRequest) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;

  let body: ProvisioningJobCreationRequest;
  try {
    body = (await request.json()) as ProvisioningJobCreationRequest;
  } catch {
    return NextResponse.json({ error: "Invalid JSON body." }, { status: 400 });
  }
  if (!body.idempotencyKey || typeof body.companyId !== "string" || !body.companyId.trim() || !body.plan) {
    return NextResponse.json({ error: "Fields `idempotencyKey`, `companyId`, and `plan` are required." }, { status: 400 });
  }
  const usesAgentSkills = Array.isArray(body.plan.rolePlans)
    && body.plan.rolePlans.some(
      (role) => role.action === "create"
        && Array.isArray(role.selectedSkills)
        && role.selectedSkills.length > 0,
    );
  if (usesAgentSkills && !serverPluginEnabled("agent-skills")) {
    return NextResponse.json(
      { error: "plugin 'agent-skills' is disabled" },
      { status: 404 },
    );
  }
  if (!(await hasGroupProvisioningCompanyAccess(auth.token, body.companyId))) {
    return NextResponse.json({ error: "Company not found." }, { status: 404 });
  }

  const runner = createGroupProvisioningRunner({
    store: await defaultGroupProvisioningStore(),
    loadDriverAvailability: getDriverAvailability,
  });
  const job = await runner.createJob({
    sessionToken: auth.token,
    actorId: auth.claims.principal_id,
    companyId: body.companyId,
    body,
  });
  return NextResponse.json({ job }, { status: 201 });
}
