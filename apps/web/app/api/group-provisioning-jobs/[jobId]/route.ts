import { NextRequest, NextResponse } from "next/server";

import { requireAuth } from "../../../../lib/api/api-auth";
import { hasGroupProvisioningCompanyAccess } from "../../../../lib/groups/group-provisioning-company-access";
import {
  createGroupProvisioningRunner,
  defaultGroupProvisioningStore,
} from "../../../../lib/groups/group-provisioning-runner";

export async function GET(request: NextRequest, context: { params: Promise<{ jobId: string }> }) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const { jobId } = await context.params;
  const runner = createGroupProvisioningRunner({ store: await defaultGroupProvisioningStore() });
  const job = await runner.getJob(jobId);
  if (!job || !(await hasGroupProvisioningCompanyAccess(auth.token, job.companyId))) {
    return NextResponse.json({ error: "Job not found." }, { status: 404 });
  }
  return NextResponse.json({ job });
}
