import { NextRequest } from "next/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { requireAuth } from "../../../lib/api/api-auth";
import { hasGroupProvisioningCompanyAccess } from "../../../lib/groups/group-provisioning-company-access";
import { createGroupProvisioningRunner, defaultGroupProvisioningStore } from "../../../lib/groups/group-provisioning-runner";
import type { GroupLaunchPlanContract } from "../../../lib/groups/group-provisioning-contract";
import { POST } from "./route";
import { GET } from "./[jobId]/route";
import { POST as POST_RUN } from "./[jobId]/run/route";
import { POST as POST_RETRY } from "./[jobId]/retry/route";
import { POST as POST_CANCEL } from "./[jobId]/cancel/route";

vi.mock("../../../lib/api/api-auth", () => ({
  requireAuth: vi.fn(),
}));

vi.mock("../../../lib/groups/group-provisioning-company-access", () => ({
  hasGroupProvisioningCompanyAccess: vi.fn(),
}));

vi.mock("../../../lib/groups/group-provisioning-runner", () => ({
  createGroupProvisioningRunner: vi.fn(),
  defaultGroupProvisioningStore: vi.fn(),
}));

describe("/api/group-provisioning-jobs", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllEnvs();
  });

  it("rejects selected skills when the agent-skills plugin is disabled", async () => {
    vi.stubEnv("CHORUZ_PLUGINS", "workspace-git,remote-ssh");
    mockAuth();
    const plan: GroupLaunchPlanContract = {
      ...jobContract().plan,
      rolePlans: [{
      slotId: "developer",
      action: "create",
      agentName: "Developer",
      roleTemplateId: "backend-engineer",
      roleTemplateVersion: "1.0.0",
      driver: "claude_terminal",
      instructionStatus: "template_default",
      setupInputs: {},
      selectedSkills: ["/tmp/example-skill"],
      workspaceMode: "generated",
      }],
    };

    const response = await POST(new NextRequest("http://localhost/api/group-provisioning-jobs", {
      method: "POST",
      body: JSON.stringify({
        idempotencyKey: "idem-1",
        companyId: "company-2",
        plan,
      }),
    }));

    expect(response.status).toBe(404);
    await expect(response.json()).resolves.toEqual({
      error: "plugin 'agent-skills' is disabled",
    });
    expect(hasGroupProvisioningCompanyAccess).not.toHaveBeenCalled();
    expect(createGroupProvisioningRunner).not.toHaveBeenCalled();
  });

  it("creates a durable job through the route wrapper", async () => {
    mockAuth();
    mockCompanyAccess(true);
    vi.mocked(defaultGroupProvisioningStore).mockResolvedValue({} as never);
    const createJob = vi.fn().mockResolvedValue(jobContract());
    vi.mocked(createGroupProvisioningRunner).mockReturnValue({ createJob } as never);

    const response = await POST(new NextRequest("http://localhost/api/group-provisioning-jobs", {
      method: "POST",
      body: JSON.stringify({
        idempotencyKey: "idem-1",
        companyId: "company-2",
        plan: jobContract().plan,
      }),
    }));

    expect(response.status).toBe(201);
    await expect(response.json()).resolves.toEqual({ job: jobContract() });
    expect(createJob).toHaveBeenCalledWith({
      sessionToken: "session-token",
      actorId: "human-1",
      companyId: "company-2",
      body: {
        idempotencyKey: "idem-1",
        companyId: "company-2",
        plan: jobContract().plan,
      },
    });
    expect(hasGroupProvisioningCompanyAccess).toHaveBeenCalledWith("session-token", "company-2");
  });

  it("rejects job creation for an inaccessible company", async () => {
    mockAuth();
    mockCompanyAccess(false);
    vi.mocked(defaultGroupProvisioningStore).mockResolvedValue({} as never);
    const createJob = vi.fn();
    vi.mocked(createGroupProvisioningRunner).mockReturnValue({ createJob } as never);

    const response = await POST(new NextRequest("http://localhost/api/group-provisioning-jobs", {
      method: "POST",
      body: JSON.stringify({
        idempotencyKey: "idem-1",
        companyId: "company-2",
        plan: jobContract().plan,
      }),
    }));

    expect(response.status).toBe(404);
    expect(createJob).not.toHaveBeenCalled();
  });

  it("reads a resumable job state for the authenticated company", async () => {
    mockAuth();
    mockCompanyAccess(true);
    vi.mocked(defaultGroupProvisioningStore).mockResolvedValue({} as never);
    const getJob = vi.fn().mockResolvedValue({ ...jobContract(), companyId: "company-2" });
    vi.mocked(createGroupProvisioningRunner).mockReturnValue({ getJob } as never);

    const response = await GET(
      new NextRequest("http://localhost/api/group-provisioning-jobs/job-1"),
      { params: Promise.resolve({ jobId: "job-1" }) },
    );

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toEqual({ job: { ...jobContract(), companyId: "company-2" } });
    expect(getJob).toHaveBeenCalledWith("job-1");
    expect(hasGroupProvisioningCompanyAccess).toHaveBeenCalledWith("session-token", "company-2");
  });

  it("checks company ownership before running a job", async () => {
    mockAuth();
    mockCompanyAccess(false);
    vi.mocked(defaultGroupProvisioningStore).mockResolvedValue({} as never);
    const getJob = vi.fn().mockResolvedValue({ ...jobContract(), companyId: "company-2" });
    const runJob = vi.fn();
    vi.mocked(createGroupProvisioningRunner).mockReturnValue({ getJob, runJob } as never);

    const response = await POST_RUN(
      new NextRequest("http://localhost/api/group-provisioning-jobs/job-1/run", {
        method: "POST",
        body: JSON.stringify({ maxSteps: 1 }),
      }),
      { params: Promise.resolve({ jobId: "job-1" }) },
    );

    expect(response.status).toBe(404);
    expect(runJob).not.toHaveBeenCalled();
  });

  it("runs an accessible job whose company differs from the session workspace", async () => {
    mockAuth();
    mockCompanyAccess(true);
    vi.mocked(defaultGroupProvisioningStore).mockResolvedValue({} as never);
    const getJob = vi.fn().mockResolvedValue({ ...jobContract(), companyId: "company-2" });
    const runJob = vi.fn().mockResolvedValue({ ...jobContract(), companyId: "company-2" });
    vi.mocked(createGroupProvisioningRunner).mockReturnValue({ getJob, runJob } as never);

    const response = await POST_RUN(
      new NextRequest("http://localhost/api/group-provisioning-jobs/job-1/run", {
        method: "POST",
        body: JSON.stringify({ maxSteps: 2 }),
      }),
      { params: Promise.resolve({ jobId: "job-1" }) },
    );

    expect(response.status).toBe(200);
    expect(runJob).toHaveBeenCalledWith({
      sessionToken: "session-token",
      actorId: "human-1",
      jobId: "job-1",
      maxSteps: 2,
    });
    expect(hasGroupProvisioningCompanyAccess).toHaveBeenCalledWith("session-token", "company-2");
  });

  it("checks company ownership before retrying a job", async () => {
    mockAuth();
    mockCompanyAccess(false);
    vi.mocked(defaultGroupProvisioningStore).mockResolvedValue({} as never);
    const getJob = vi.fn().mockResolvedValue({ ...jobContract(), companyId: "company-2" });
    const retryJob = vi.fn();
    vi.mocked(createGroupProvisioningRunner).mockReturnValue({ getJob, retryJob } as never);

    const response = await POST_RETRY(
      new NextRequest("http://localhost/api/group-provisioning-jobs/job-1/retry", {
        method: "POST",
        body: JSON.stringify({ choice: "retry_validation" }),
      }),
      { params: Promise.resolve({ jobId: "job-1" }) },
    );

    expect(response.status).toBe(404);
    expect(retryJob).not.toHaveBeenCalled();
  });

  it("drops caller-supplied retry status overrides", async () => {
    mockAuth();
    mockCompanyAccess(true);
    vi.mocked(defaultGroupProvisioningStore).mockResolvedValue({} as never);
    const getJob = vi.fn().mockResolvedValue({ ...jobContract(), companyId: "company-2" });
    const retryJob = vi.fn().mockResolvedValue(jobContract());
    vi.mocked(createGroupProvisioningRunner).mockReturnValue({ getJob, retryJob } as never);

    const response = await POST_RETRY(
      new NextRequest("http://localhost/api/group-provisioning-jobs/job-1/retry", {
        method: "POST",
        body: JSON.stringify({ choice: "retry_validation", nextStatus: "creating_group" }),
      }),
      { params: Promise.resolve({ jobId: "job-1" }) },
    );

    expect(response.status).toBe(200);
    expect(retryJob).toHaveBeenCalledWith("job-1", { choice: "retry_validation" });
    expect(hasGroupProvisioningCompanyAccess).toHaveBeenCalledWith("session-token", "company-2");
  });

  it("checks company ownership before canceling a job", async () => {
    mockAuth();
    mockCompanyAccess(false);
    vi.mocked(defaultGroupProvisioningStore).mockResolvedValue({} as never);
    const getJob = vi.fn().mockResolvedValue({ ...jobContract(), companyId: "company-2" });
    const cancelJob = vi.fn();
    vi.mocked(createGroupProvisioningRunner).mockReturnValue({ getJob, cancelJob } as never);

    const response = await POST_CANCEL(
      new NextRequest("http://localhost/api/group-provisioning-jobs/job-1/cancel", {
        method: "POST",
        body: JSON.stringify({ choice: "cancel_only" }),
      }),
      { params: Promise.resolve({ jobId: "job-1" }) },
    );

    expect(response.status).toBe(404);
    expect(cancelJob).not.toHaveBeenCalled();
  });

  it("cancels an accessible job whose company differs from the session workspace", async () => {
    mockAuth();
    mockCompanyAccess(true);
    vi.mocked(defaultGroupProvisioningStore).mockResolvedValue({} as never);
    const getJob = vi.fn().mockResolvedValue({ ...jobContract(), companyId: "company-2" });
    const cancelJob = vi.fn().mockResolvedValue({ ...jobContract(), companyId: "company-2", status: "canceled" });
    vi.mocked(createGroupProvisioningRunner).mockReturnValue({ getJob, cancelJob } as never);

    const response = await POST_CANCEL(
      new NextRequest("http://localhost/api/group-provisioning-jobs/job-1/cancel", {
        method: "POST",
        body: JSON.stringify({ choice: "cancel_only" }),
      }),
      { params: Promise.resolve({ jobId: "job-1" }) },
    );

    expect(response.status).toBe(200);
    expect(cancelJob).toHaveBeenCalledWith("job-1", { choice: "cancel_only" });
    expect(hasGroupProvisioningCompanyAccess).toHaveBeenCalledWith("session-token", "company-2");
  });
});

function mockAuth() {
  vi.mocked(requireAuth).mockResolvedValue({
    token: "session-token",
    claims: {
      principal_id: "human-1",
      workspace_id: "company-1",
      display_name: "Alice",
      expires_at_epoch_s: 1,
    },
  });
}

function mockCompanyAccess(accessible: boolean) {
  vi.mocked(hasGroupProvisioningCompanyAccess).mockResolvedValue(accessible);
}

function jobContract() {
  return {
    id: "job-1",
    status: "validating" as const,
    companyId: "company-1",
    requestedBy: "human-1",
    idempotencyKey: "idem-1",
    plan: {
      groupTemplateId: "software-development-team",
      groupTemplateVersion: "1.0.0" as const,
      groupName: "onboarding-mvp",
      mission: "Ship onboarding MVP",
      kickoffText: "Mission: Ship onboarding MVP\n\nRoles: Project Operator.\n\nNext user action: send the first concrete work item or question when ready.",
      startWorkMode: "manual" as const,
      rolePlans: [],
    },
    progressSteps: [],
    stepResults: [],
    issues: [],
    recoveryChoices: [],
    allowedUiActions: ["read_job", "run", "cancel"] as const,
    allowedBackendActions: ["read_job", "advance_job", "cancel_job"] as const,
    createdAgentIds: [],
    reusedAgentIds: [],
    createdAt: "2026-05-19T00:00:00.000Z",
    updatedAt: "2026-05-19T00:00:00.000Z",
  };
}
