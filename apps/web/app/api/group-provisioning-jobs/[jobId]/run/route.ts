import { NextRequest, NextResponse } from "next/server";

import { requireAuth } from "../../../../../lib/api/api-auth";
import { hasGroupProvisioningCompanyAccess } from "../../../../../lib/groups/group-provisioning-company-access";
import {
  createGroupProvisioningRunner,
  defaultGroupProvisioningStore,
} from "../../../../../lib/groups/group-provisioning-runner";
import { getDriverAvailability } from "../../../../../lib/drivers/driver-availability";
import type { ProvisioningJobRunRequest } from "../../../../../lib/groups/group-provisioning-contract";

export async function POST(request: NextRequest, context: { params: Promise<{ jobId: string }> }) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const { jobId } = await context.params;
  let body: ProvisioningJobRunRequest = {};
  try {
    body = (await request.json()) as ProvisioningJobRunRequest;
  } catch {}
  const runner = createGroupProvisioningRunner({
    store: await defaultGroupProvisioningStore(),
    loadDriverAvailability: getDriverAvailability,
  });
  const existing = await runner.getJob(jobId);
  if (!existing || !(await hasGroupProvisioningCompanyAccess(auth.token, existing.companyId))) {
    return NextResponse.json({ error: "Job not found." }, { status: 404 });
  }
  const job = await runner.runJob({
    sessionToken: auth.token,
    actorId: auth.claims.principal_id,
    jobId,
    maxSteps: body.maxSteps,
  });
  if (!job) {
    return NextResponse.json({ error: "Job not found." }, { status: 404 });
  }
  return NextResponse.json({ job });
}
