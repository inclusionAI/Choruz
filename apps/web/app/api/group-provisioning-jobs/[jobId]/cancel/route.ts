import { NextRequest, NextResponse } from "next/server";

import { requireAuth } from "../../../../../lib/api/api-auth";
import { batchDisableAgents, fetchConsoleSnapshot } from "../../../../../lib/api/choruz-api";
import { hasGroupProvisioningCompanyAccess } from "../../../../../lib/groups/group-provisioning-company-access";
import {
  createGroupProvisioningRunner,
  defaultGroupProvisioningStore,
} from "../../../../../lib/groups/group-provisioning-runner";
import type { ProvisioningJobCancelRequest } from "../../../../../lib/groups/group-provisioning-contract";

export async function POST(request: NextRequest, context: { params: Promise<{ jobId: string }> }) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const { jobId } = await context.params;
  let body: ProvisioningJobCancelRequest;
  try {
    body = (await request.json()) as ProvisioningJobCancelRequest;
  } catch {
    return NextResponse.json({ error: "Invalid JSON body." }, { status: 400 });
  }
  if (body.choice !== "cancel_only" && body.choice !== "soft_delete_generated_agents") {
    return NextResponse.json({ error: "Field `choice` must be `cancel_only` or `soft_delete_generated_agents`." }, { status: 400 });
  }
  const runner = createGroupProvisioningRunner({
    store: await defaultGroupProvisioningStore(),
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
  const job = await runner.cancelJob(jobId, body);
  if (!job) {
    return NextResponse.json({ error: "Job not found." }, { status: 404 });
  }
  return NextResponse.json({ job });
}
