import { NextRequest, NextResponse } from "next/server";

import { requireAuth } from "../../../../../lib/api/api-auth";
import { batchDisableAgents, fetchConsoleSnapshot } from "../../../../../lib/api/choruz-api";
import { getDriverAvailability } from "../../../../../lib/drivers/driver-availability";
import { hasGroupProvisioningCompanyAccess } from "../../../../../lib/groups/group-provisioning-company-access";
import {
  createGroupProvisioningRunner,
  defaultGroupProvisioningStore,
} from "../../../../../lib/groups/group-provisioning-runner";
import type { ProvisioningJobRetryRequest } from "../../../../../lib/groups/group-provisioning-contract";

export async function POST(request: NextRequest, context: { params: Promise<{ jobId: string }> }) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const { jobId } = await context.params;
  let body: ProvisioningJobRetryRequest;
  try {
    body = (await request.json()) as ProvisioningJobRetryRequest;
  } catch {
    return NextResponse.json({ error: "Invalid JSON body." }, { status: 400 });
  }
  if (!body.choice) {
    return NextResponse.json({ error: "Field `choice` is required." }, { status: 400 });
  }
  const runner = createGroupProvisioningRunner({
    store: await defaultGroupProvisioningStore(),
    loadDriverAvailability: getDriverAvailability,
    loadGeneratedAgentCleanupCandidates: async (input) => {
      const names = new Set(input.agentNames.map((name) => name.trim().toLowerCase()));
      const snapshot = await fetchConsoleSnapshot(auth.token);
      return snapshot.agents
        .filter((agent) => names.has(agent.name.trim().toLowerCase()))
        .map((agent) => ({
          principal: agent,
          runtimeBinding: null,
          companyId: agent.workspace_id,
          workspaceId: agent.workspace_id,
        }));
    },
    softDisableGeneratedAgents: async (input) => {
      await batchDisableAgents(auth.token, input.actorId, input.agentIds);
    },
  });
  const existing = await runner.getJob(jobId);
  if (!existing || !(await hasGroupProvisioningCompanyAccess(auth.token, existing.companyId))) {
    return NextResponse.json({ error: "Job not found." }, { status: 404 });
  }
  const job = await runner.retryJob(jobId, {
    choice: body.choice,
    ...(body.roleSlotId ? { roleSlotId: body.roleSlotId } : {}),
  });
  if (!job) {
    return NextResponse.json({ error: "Job not found." }, { status: 404 });
  }
  return NextResponse.json({ job });
}
