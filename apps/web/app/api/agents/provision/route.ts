import { NextRequest, NextResponse } from "next/server";

import { requireAuth } from "../../../../lib/api/api-auth";
import {
  AgentProvisioningError,
  defaultRoleTemplateProvenanceWriter,
  provisionAgent,
  validateCustomWorkspacePath,
  validateProvisionRequestBody,
  type ProvisionRequestBody,
} from "../../../../lib/agents/agent-provisioning";
import {
  ProvisioningIdempotencyConflictError,
  withProvisioningIdempotency,
} from "../../../../lib/agents/agent-provisioning-idempotency";
import { serverPluginEnabled } from "../../../../plugins/server-plugin";

export async function POST(request: NextRequest) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;
  const { token: sessionToken, claims } = auth;
  const actorId = claims.principal_id;

  let body: ProvisionRequestBody;
  try {
    body = (await request.json()) as ProvisionRequestBody;
  } catch {
    return NextResponse.json(
      { error: "Invalid JSON body." },
      { status: 400 },
    );
  }

  const allowChannelVisibility = isInternalProvisionRequest(request);
  const validationError = validateProvisionRequestBody(body, {
    allowChannelVisibility,
  });
  if (validationError) {
    return NextResponse.json({ error: validationError }, { status: 400 });
  }
  if (body.skill_paths?.length && !serverPluginEnabled("agent-skills")) {
    return NextResponse.json(
      { error: "plugin 'agent-skills' is disabled" },
      { status: 404 },
    );
  }
  if (body.driver_type === "mathcode_terminal" && !serverPluginEnabled("mathcode")) {
    return NextResponse.json(
      { error: "plugin 'mathcode' is disabled" },
      { status: 404 },
    );
  }

  const workspacePathError = validateCustomWorkspacePath(body.workspace_path);
  if (workspacePathError) {
    return NextResponse.json(
      { error: workspacePathError.error },
      { status: workspacePathError.status },
    );
  }

  const { channel_visibility: _ignoredChannelVisibility, ...publicBody } = body;

  try {
    const provisionBody = allowChannelVisibility ? body : publicBody;
    const result = await withProvisioningIdempotency(actorId, provisionBody, (idempotency) =>
      provisionAgent({
        sessionToken,
        actorId,
        body: provisionBody,
        ...idempotency,
        ...(body.template_metadata
          ? { provenanceWriter: defaultRoleTemplateProvenanceWriter }
          : {}),
      }),
    );
    return NextResponse.json(result, { status: 201 });
  } catch (error) {
    if (error instanceof ProvisioningIdempotencyConflictError) {
      return NextResponse.json({ error: error.message }, { status: 409 });
    }
    const detail =
      error instanceof AgentProvisioningError
        ? error.detail.message
        : error instanceof Error
          ? error.message
          : "Provisioning failed";
    return NextResponse.json({ error: detail }, { status: 500 });
  }
}

function isInternalProvisionRequest(request: NextRequest): boolean {
  const expected = process.env.CHORUZ_INTERNAL_PROVISION_TOKEN;
  if (!expected) return false;
  return request.headers.get("x-choruz-internal-provision-token") === expected;
}
