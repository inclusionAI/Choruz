import { describe, expect, it, vi } from "vitest";

import {
  createGroupProvisioningRunner,
  type GroupProvisioningStore,
} from "./group-provisioning-runner";
import type { AgentProvisioningInput } from "../agents/agent-provisioning";
import type {
  GroupLaunchPlanContract,
  ProvisioningJobCreationRequest,
} from "./group-provisioning-contract";
import type { ExistingAgentCandidate } from "./team-template-validation";
import {
  sanitizeProvisioningJson,
  type CreateGroupProvisioningJobInput,
  type GroupProvisioningJob,
  type GroupProvisioningJobStatus,
  type JsonValue,
  type UpdateGroupProvisioningJobInput,
} from "./group-provisioning-store";

const basePlan: GroupLaunchPlanContract = {
  groupTemplateId: "software-development-team",
  groupTemplateVersion: "1.0.0",
  groupName: "onboarding-mvp",
  mission: "Ship onboarding MVP",
  kickoffText: "Mission: Ship onboarding MVP\n\nWorkflow: Plan -> implement -> review -> verify -> summarize.\n\nPlease wait for the user to provide the first concrete work item before starting execution.\n\nRoles: Project Operator, Backend Engineer, Code Reviewer.\n\nNext user action: send the first concrete work item or question when ready. Until then, work waits for the user kickoff.",
  startWorkMode: "manual",
  workflow: {
    coordinatorRoleSlotId: "project-operator",
    participantRoleDefaults: {
      "project-operator": ["coordinator"],
      "backend-engineer": ["owner"],
      "code-reviewer": ["quality_check"],
      "frontend-engineer": ["owner"],
    },
  },
  rolePlans: [
    {
      slotId: "project-operator",
      action: "create",
      agentName: "project-operator",
      roleTemplateId: "project-operator",
      roleTemplateVersion: "1.0.0",
      driver: "codex_terminal",
      instructionStatus: "group_context_added",
      setupInputs: {},
      selectedSkills: [],
      workspaceMode: "none",
    },
    {
      slotId: "backend-engineer",
      action: "create",
      agentName: "backend-engineer",
      roleTemplateId: "backend-engineer",
      roleTemplateVersion: "1.0.0",
      driver: "codex_terminal",
      instructionStatus: "group_context_added",
      setupInputs: {},
      selectedSkills: [],
      workspaceMode: "none",
    },
    {
      slotId: "code-reviewer",
      action: "create",
      agentName: "code-reviewer",
      roleTemplateId: "code-reviewer",
      roleTemplateVersion: "1.0.0",
      driver: "codex_terminal",
      instructionStatus: "group_context_added",
      setupInputs: {},
      selectedSkills: [],
      workspaceMode: "none",
    },
    {
      slotId: "frontend-engineer",
      action: "skip",
      roleTemplateId: "frontend-engineer",
      roleTemplateVersion: "1.0.0",
      reason: "user_choice",
    },
  ],
};

const provisionedKickoffText = "Mission: Ship onboarding MVP\n\nWorkflow: Plan -> implement -> review -> verify -> summarize.\n\nPlease wait for the user to provide the first concrete work item before starting execution.\n\nCurrent members: project-operator (Project Operator), backend-engineer (Backend Engineer), code-reviewer (Code Reviewer).\n\nNext user action: send the first concrete work item or question when ready. Until then, work waits for the user kickoff.";

describe("group provisioning runner", () => {
  it("creates a durable validation job without provisioning side effects", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn();
    const createGroup = vi.fn();
    const runner = createGroupProvisioningRunner({ store, newId: ids("job-1"), provisionAgent, createGroup });

    const job = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: basePlan }));

    expect(job.status).toBe("validating");
    expect(job.allowedBackendActions).toContain("advance_job");
    expect(store.jobs).toHaveLength(1);
    expect(provisionAgent).not.toHaveBeenCalled();
    expect(createGroup).not.toHaveBeenCalled();
  });

  it("blocks unknown reused agents during job creation before side effects", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn();
    const createGroup = vi.fn();
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", "lease-1"),
      provisionAgent,
      createGroup,
      loadExistingAgentCandidates: vi.fn().mockResolvedValue([]),
    });

    const job = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: reusePlan() }));

    expect(job.status).toBe("failed_validation");
    expect(job.issues.map((issue) => issue.code)).toContain("unknown_existing_agent");
    expect(provisionAgent).not.toHaveBeenCalled();
    expect(createGroup).not.toHaveBeenCalled();
  });

  it("treats stale reused-agent warnings as blocking before side effects", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn();
    const createGroup = vi.fn();
    const candidate = existingAgentCandidate();
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", "lease-1"),
      provisionAgent,
      createGroup,
      loadExistingAgentCandidates: vi.fn().mockResolvedValue([
        {
          ...candidate,
          runtimeBinding: { ...candidate.runtimeBinding!, state: "error" },
        },
      ]),
    });

    const job = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: reusePlan() }));

    expect(job.status).toBe("failed_validation");
    expect(job.issues).toEqual(expect.arrayContaining([
      expect.objectContaining({ code: "runtime_binding_not_reusable", severity: "error" }),
    ]));
    expect(provisionAgent).not.toHaveBeenCalled();
    expect(createGroup).not.toHaveBeenCalled();
  });

  it("revalidates reused agent ownership before advancing from validation", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn();
    const createGroup = vi.fn();
    const candidates = [existingAgentCandidate()];
    const loadExistingAgentCandidates = vi.fn(async () => candidates.splice(0));
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", "lease-1"),
      provisionAgent,
      createGroup,
      loadExistingAgentCandidates,
    });
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: reusePlan() }));

    const failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });

    expect(failed?.status).toBe("failed_validation");
    expect(failed?.issues.map((issue) => issue.code)).toContain("unknown_existing_agent");
    expect(provisionAgent).not.toHaveBeenCalled();
    expect(createGroup).not.toHaveBeenCalled();
  });

  it("advances exactly one safe step by default and resumes from refreshed state", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn().mockResolvedValue(agentResponse("agent-operator", "project-operator"));
    const runner = createGroupProvisioningRunner({ store, newId: ids("job-1", "lease-1", "lease-2"), provisionAgent });
    const plan: GroupLaunchPlanContract = {
      ...basePlan,
      rolePlans: basePlan.rolePlans.map((rolePlan) =>
        rolePlan.slotId === "project-operator" && rolePlan.action === "create"
          ? {
              ...rolePlan,
              harnessAccountId: "12345678-1234-1234-1234-123456789abc",
              model: "gpt-5.6-sol",
            }
          : rolePlan,
      ),
    };
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan }));

    const afterValidation = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });
    const refreshed = await runner.getJob(created.id);
    const afterFirstAgent = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });

    expect(afterValidation?.status).toBe("creating_agents");
    expect(refreshed?.status).toBe("creating_agents");
    expect(afterFirstAgent?.stepResults).toHaveLength(1);
    expect(afterFirstAgent?.stepResults[0]).toMatchObject({ kind: "created_agent", roleSlotId: "project-operator" });
    expect(provisionAgent).toHaveBeenCalledTimes(1);
    expect(provisionAgent).toHaveBeenCalledWith(expect.objectContaining({
      body: expect.objectContaining({
        harness_account_id: "12345678-1234-1234-1234-123456789abc",
        model: "gpt-5.6-sol",
      }),
    }));
  });

  it("fails validation before side effects when the selected driver is unavailable", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn();
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", "lease-1"),
      provisionAgent,
      loadDriverAvailability: async () => [{
        driverId: "codex_terminal",
        label: "Codex",
        status: "unavailable",
        available: false,
        reason: "Codex CLI was not found.",
        setupHint: "Install Codex.",
      }],
    });

    const job = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: basePlan }));

    expect(job.status).toBe("failed_validation");
    expect(job.issues).toEqual(expect.arrayContaining([
      expect.objectContaining({ severity: "error", code: "driver_unavailable", roleSlotId: "project-operator" }),
    ]));
    expect(provisionAgent).not.toHaveBeenCalled();
  });

  it("passes webhook role configuration through validation into provisioning", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn().mockResolvedValue(agentResponse("agent-operator", "project-operator"));
    const runner = createGroupProvisioningRunner({ store, newId: ids("job-1", "lease-1", "lease-2"), provisionAgent });
    const plan: GroupLaunchPlanContract = {
      ...basePlan,
      rolePlans: basePlan.rolePlans.map((rolePlan) =>
        rolePlan.slotId === "project-operator" && rolePlan.action === "create"
          ? {
              ...rolePlan,
              driver: "webhook_agent",
              webhookConfigured: true,
              webhookUrl: "https://example.com/hook",
            }
          : rolePlan
      ),
    };
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan }));

    const afterValidation = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });
    const afterFirstAgent = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });

    expect(afterValidation?.status).toBe("creating_agents");
    expect(afterValidation?.issues).toEqual(expect.arrayContaining([
      expect.objectContaining({ severity: "warning", code: "incompatible_driver", roleSlotId: "project-operator" }),
    ]));
    expect(afterFirstAgent?.stepResults[0]).toMatchObject({ kind: "created_agent", roleSlotId: "project-operator" });
    expect(provisionAgent).toHaveBeenCalledWith(expect.objectContaining({
      body: expect.objectContaining({
        driver_type: "webhook_agent",
        webhook_url: "https://example.com/hook",
      }),
    }));
    expect(provisionAgent.mock.calls[0]?.[0].body).not.toHaveProperty("instructions");
    expect(provisionAgent.mock.calls[0]?.[0].body).not.toHaveProperty("webhook_secret");
  });

  it("recovers webhook agent creation when retry replays a recorded webhook registration", async () => {
    const store = new MemoryStore();
    let failAfterWebhookRegistration = true;
    const provisionAgent = vi.fn(async (input: AgentProvisioningInput) => {
      const context = input.jobContext!;
      const registerWebhookStep = {
        key: `${context.jobId}:${context.roleSlotId}:${context.idempotencyKey}:register_webhook`,
        step: "register_webhook" as const,
        jobId: context.jobId,
        roleSlotId: context.roleSlotId,
        idempotencyKey: context.idempotencyKey,
      };
      const existingWebhook = await input.stepRecorder?.readStep<{ registered: boolean }>(registerWebhookStep);
      if (existingWebhook && !context.allowWebhookRegistrationReplayWithoutOutput) {
        throw new Error("generated signing secret cannot be replayed");
      }
      if (!existingWebhook) {
        await input.stepRecorder?.recordStep({
          ...registerWebhookStep,
          output: {
            agentId: "agent-operator",
            registered: true,
            eventTypes: ["app_mention"],
          },
        });
      }
      if (failAfterWebhookRegistration) {
        failAfterWebhookRegistration = false;
        throw new Error("failed after webhook registration");
      }
      return agentResponse("agent-operator", "project-operator");
    });
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", "lease-1", "lease-2", "lease-3"),
      provisionAgent,
    });
    const plan: GroupLaunchPlanContract = {
      ...basePlan,
      rolePlans: basePlan.rolePlans.map((rolePlan) =>
        rolePlan.slotId === "project-operator" && rolePlan.action === "create"
          ? {
              ...rolePlan,
              driver: "webhook_agent",
              webhookConfigured: true,
              webhookUrl: "https://example.com/hook",
            }
          : rolePlan
      ),
    };
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan }));
    await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });
    const failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });

    await runner.retryJob(created.id, { choice: "retry_agent_creation" });
    const recovered = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });

    expect(failed?.status).toBe("partial_failure");
    expect(recovered?.stepResults[0]).toMatchObject({ kind: "created_agent", agentId: "agent-operator" });
    expect(provisionAgent).toHaveBeenCalledTimes(2);
    expect(provisionAgent.mock.calls[0]?.[0].body).not.toHaveProperty("webhook_secret");
    expect(provisionAgent.mock.calls[1]?.[0].body).not.toHaveProperty("webhook_secret");
  });

  it("recovers stale leases before running a job", async () => {
    const store = new MemoryStore();
    const runner = createGroupProvisioningRunner({
      store,
      now: dates("2026-05-19T00:00:00.000Z", "2026-05-19T00:01:01.000Z", "2026-05-19T00:01:01.000Z"),
      newId: ids("job-1", "lease-a", "lease-b"),
      leaseMs: 1_000,
    });
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: basePlan }));
    await store.acquireLease({
      jobId: created.id,
      leaseOwner: "tab-a",
      leaseToken: "old-lease",
      leaseMs: 1_000,
      now: new Date("2026-05-19T00:00:00.000Z"),
    });

    const afterRun = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });

    expect(afterRun?.status).toBe("creating_agents");
    expect(store.jobs[0].leaseToken).toBeNull();
  });

  it("prepares idempotent retry from a failed agent creation phase", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn().mockRejectedValueOnce(new Error("driver unavailable"));
    const runner = createGroupProvisioningRunner({ store, newId: ids("job-1", "lease-1", "lease-2"), provisionAgent });
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: basePlan }));
    await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });
    const failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });

    const retried = await runner.retryJob(created.id, { choice: "retry_agent_creation" });

    expect(failed?.status).toBe("partial_failure");
    expect(retried?.status).toBe("creating_agents");
    expect(retried?.issues).toEqual([]);
  });

  it("derives retry status from the recovery choice instead of caller-supplied status", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn().mockRejectedValueOnce(new Error("driver unavailable"));
    const runner = createGroupProvisioningRunner({ store, newId: ids("job-1", "lease-1", "lease-2"), provisionAgent });
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: basePlan }));
    await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });
    await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });

    const retried = await runner.retryJob(created.id, { choice: "retry_agent_creation", nextStatus: "adding_members" });

    expect(retried?.status).toBe("creating_agents");
  });

  it("persists internal agent provisioning step records across runner retries", async () => {
    const store = new MemoryStore();
    let principalCreates = 0;
    let failAfterPrincipal = true;
    const provisionAgent = vi.fn(async (input: AgentProvisioningInput) => {
      const context = input.jobContext!;
      const principalStep = {
        key: `${context.jobId}:${context.roleSlotId}:${context.idempotencyKey}:create_principal`,
        step: "create_principal" as const,
        ...context,
      };
      const existingPrincipal = await input.stepRecorder?.readStep(principalStep);
      if (!existingPrincipal) {
        principalCreates += 1;
        await input.stepRecorder?.recordStep({
          ...principalStep,
          output: { agent: { id: "agent-operator" } },
        });
      }
      if (failAfterPrincipal) {
        failAfterPrincipal = false;
        throw new Error("failed after principal creation");
      }
      return agentResponse("agent-operator", "project-operator");
    });
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", "lease-1", "lease-2", "lease-3"),
      provisionAgent,
    });
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: basePlan }));
    await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });
    const failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });
    await runner.retryJob(created.id, { choice: "retry_agent_creation" });
    const recovered = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });

    expect(failed?.status).toBe("partial_failure");
    expect(recovered?.stepResults[0]).toMatchObject({ kind: "created_agent", agentId: "agent-operator" });
    expect(principalCreates).toBe(1);
    expect(provisionAgent).toHaveBeenCalledTimes(2);
  });

  it("replays completed jobs without side effects", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn()
      .mockResolvedValueOnce(agentResponse("agent-operator", "project-operator"))
      .mockResolvedValueOnce(agentResponse("agent-backend", "backend-engineer"))
      .mockResolvedValueOnce(agentResponse("agent-reviewer", "code-reviewer"));
    const createGroup = vi.fn().mockResolvedValue(groupResponse("group-1"));
    const addGroupMembers = vi.fn().mockResolvedValue(groupResponse("group-1"));
    const sendMessage = vi.fn().mockResolvedValue(messageResponse("message-1"));
    const enableRoutingPolicy = vi.fn().mockResolvedValue(undefined);
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", ...Array.from({ length: 20 }, (_, index) => `lease-${index}`)),
      provisionAgent,
      createGroup,
      addGroupMembers,
      sendMessage,
      enableRoutingPolicy,
    });
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: basePlan }));
    let completed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    while (completed?.status !== "completed") {
      completed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    }
    const replayed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });

    expect(completed?.status).toBe("completed");
    expect(replayed?.status).toBe("completed");
    expect(provisionAgent).toHaveBeenCalledTimes(3);
    expect(createGroup).toHaveBeenCalledTimes(1);
    expect(addGroupMembers).toHaveBeenCalledTimes(3);
    expect(enableRoutingPolicy).toHaveBeenCalledTimes(1);
    expect(enableRoutingPolicy).toHaveBeenCalledWith(expect.objectContaining({
      conversationId: "group-1",
      coordinatorAgentId: "agent-operator",
    }));
    expect(sendMessage).toHaveBeenCalledTimes(1);
    expect(sendMessage).toHaveBeenCalledWith(
      "token",
      "human-1",
      "group-1",
      provisionedKickoffText,
      { source: "group_provisioning", job_id: "job-1", passive: true },
      "text",
      "group-provisioning:job-1:idem-1:kickoff:group-1",
    );
  });


  it("re-persists group metadata idempotently after routing-policy retry", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn()
      .mockResolvedValueOnce(agentResponse("agent-operator", "project-operator"))
      .mockResolvedValueOnce(agentResponse("agent-backend", "backend-engineer"))
      .mockResolvedValueOnce(agentResponse("agent-reviewer", "code-reviewer"));
    const enableRoutingPolicy = vi.fn()
      .mockRejectedValueOnce(new Error("routing unavailable"))
      .mockResolvedValueOnce(undefined);
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", ...Array.from({ length: 20 }, (_, index) => `lease-${index}`)),
      provisionAgent,
      createGroup: vi.fn().mockResolvedValue(groupResponse("group-1")),
      addGroupMembers: vi.fn().mockResolvedValue(groupResponse("group-1")),
      enableRoutingPolicy,
      sendMessage: vi.fn().mockResolvedValue(messageResponse("message-1")),
    });
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: basePlan }));
    let failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    for (let attempt = 0; failed?.status !== "partial_failure" && attempt < 10; attempt += 1) {
      failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    }

    expect(failed?.status).toBe("partial_failure");
    expect(failed?.issues.at(-1)).toMatchObject({ code: "routing_policy_failed" });
    expect(store.groupTemplateInstances).toHaveLength(1);
    expect(store.roleAssignments).toHaveLength(4);

    let retried = await runner.retryJob(created.id, { choice: "retry_member_add" });
    for (let attempt = 0; retried?.status !== "completed" && attempt < 10; attempt += 1) {
      if (retried && ["failed", "failed_validation", "partial_failure", "rolled_back", "canceled"].includes(retried.status)) break;
      retried = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    }

    expect(retried?.status).toBe("completed");
    expect(enableRoutingPolicy).toHaveBeenCalledTimes(2);
    expect(store.groupTemplateInstances).toHaveLength(1);
    expect(store.roleAssignments).toHaveLength(4);
    expect(store.groupTemplateInstances[0]).toMatchObject({ groupConversationId: "group-1" });
    expect(store.roleAssignments.map((assignment) => (assignment as { id: string }).id).sort()).toEqual([
      "group-provisioning:job-1:idem-1:assignment:backend-engineer",
      "group-provisioning:job-1:idem-1:assignment:code-reviewer",
      "group-provisioning:job-1:idem-1:assignment:frontend-engineer",
      "group-provisioning:job-1:idem-1:assignment:project-operator",
    ]);
  });

  it("posts kickoff from actual added members when optional software team roles are skipped", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn()
      .mockResolvedValueOnce(agentResponse("agent-operator", "project-operator"))
      .mockResolvedValueOnce(agentResponse("agent-backend", "backend-engineer"))
      .mockResolvedValueOnce(agentResponse("agent-reviewer", "code-reviewer"));
    const createGroup = vi.fn().mockResolvedValue(groupResponse("group-1"));
    const addGroupMembers = vi.fn().mockResolvedValue(undefined);
    const sendMessage = vi.fn().mockResolvedValue(messageResponse("msg-1"));
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", "lease-1", "lease-2", "lease-3", "lease-4", "lease-5", "lease-6", "lease-7", "lease-8", "lease-9"),
      provisionAgent,
      createGroup,
      addGroupMembers,
      enableRoutingPolicy: vi.fn().mockResolvedValue(undefined),
      sendMessage,
    });
    const plan: GroupLaunchPlanContract = {
      ...basePlan,
      rolePlans: [
        ...basePlan.rolePlans.slice(0, 3),
        {
          slotId: "frontend-engineer",
          action: "skip",
          roleTemplateId: "frontend-engineer",
          roleTemplateVersion: "1.0.0",
          reason: "user_choice",
        },
        {
          slotId: "qa-tester",
          action: "skip",
          roleTemplateId: "qa-tester",
          roleTemplateVersion: "1.0.0",
          reason: "user_choice",
        },
        {
          slotId: "devops-engineer",
          action: "skip",
          roleTemplateId: "devops-engineer",
          roleTemplateVersion: "1.0.0",
          reason: "user_choice",
        },
      ],
    };
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan }));
    let completed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    while (completed?.status !== "completed") {
      completed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    }

    expect(completed.stepResults).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: "skipped_optional_role", roleSlotId: "frontend-engineer" }),
      expect.objectContaining({ kind: "skipped_optional_role", roleSlotId: "qa-tester" }),
      expect.objectContaining({ kind: "skipped_optional_role", roleSlotId: "devops-engineer" }),
      expect.objectContaining({ kind: "kickoff_post", kickoffText: provisionedKickoffText }),
    ]));
    expect(completed?.plan.workflow?.participantRoleDefaults).toEqual({
      "project-operator": ["coordinator"],
      "backend-engineer": ["owner"],
      "code-reviewer": ["quality_check"],
    });
    expect(completed?.plan.workflow?.participantRoleDefaults).not.toHaveProperty("frontend-engineer");

    const persistedWorkflow = (store.groupTemplateInstances[0] as {
      workflow: { participantRoleDefaults: Record<string, string[]> };
    }).workflow;
    expect(persistedWorkflow.participantRoleDefaults).toEqual({
      "project-operator": ["coordinator"],
      "backend-engineer": ["owner"],
      "code-reviewer": ["quality_check"],
    });
    for (const skippedRole of ["frontend-engineer", "qa-tester", "devops-engineer"]) {
      expect(completed.plan.workflow?.participantRoleDefaults).not.toHaveProperty(skippedRole);
      expect(persistedWorkflow.participantRoleDefaults).not.toHaveProperty(skippedRole);
    }
    expect(sendMessage).toHaveBeenCalledWith(
      "token",
      "human-1",
      "group-1",
      provisionedKickoffText,
      { source: "group_provisioning", job_id: "job-1", passive: true },
      "text",
      "group-provisioning:job-1:idem-1:kickoff:group-1",
    );
    const postedText = sendMessage.mock.calls[0][3] as string;
    expect(postedText).toContain("Current members: project-operator (Project Operator), backend-engineer (Backend Engineer), code-reviewer (Code Reviewer).");
    expect(postedText).not.toMatch(/\b(frontend-engineer|qa-tester|devops-engineer)\b/i);
    expect(postedText).not.toMatch(/\b(Frontend Engineer|QA Tester|DevOps Engineer)\b/);
  });

  it("uses reused agent display names in the posted kickoff", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn()
      .mockResolvedValueOnce(agentResponse("agent-backend", "backend-engineer"))
      .mockResolvedValueOnce(agentResponse("agent-reviewer", "code-reviewer"));
    const sendMessage = vi.fn().mockResolvedValue(messageResponse("msg-1"));
    const loadExistingAgentCandidates = vi.fn().mockResolvedValue([
      existingAgentCandidate({
        principal: {
          ...existingAgentCandidate().principal,
          name: "@Reusable\n\tOperator",
        },
      }),
    ]);
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", ...Array.from({ length: 14 }, (_, index) => `lease-${index}`)),
      provisionAgent,
      createGroup: vi.fn().mockResolvedValue(groupResponse("group-1")),
      addGroupMembers: vi.fn().mockResolvedValue(undefined),
      enableRoutingPolicy: vi.fn().mockResolvedValue(undefined),
      sendMessage,
      loadExistingAgentCandidates,
    });
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: reusePlan() }));
    expect(created.plan.rolePlans[0]).toEqual(expect.objectContaining({
      action: "reuse",
      displayName: "Reusable Operator",
    }));
    let completed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    while (completed?.status !== "completed") {
      completed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    }

    const postedText = sendMessage.mock.calls[0][3] as string;
    expect(completed.stepResults).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: "reused_agent", agentId: "agent-operator", agentName: "Reusable Operator" }),
    ]));
    expect(postedText).toContain("Reusable Operator (Project Operator)");
    expect(postedText).not.toContain("@Reusable");
    expect(postedText).not.toContain("agent-operator (Project Operator)");
    expect(loadExistingAgentCandidates).toHaveBeenCalledTimes(2);
  });

  it("does not infer a coordinator from role or agent names without explicit workflow metadata", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn()
      .mockResolvedValueOnce(agentResponse("agent-operator", "project-operator"))
      .mockResolvedValueOnce(agentResponse("agent-backend", "backend-engineer"))
      .mockResolvedValueOnce(agentResponse("agent-reviewer", "code-reviewer"));
    const enableRoutingPolicy = vi.fn().mockResolvedValue(undefined);
    const planWithoutWorkflow: GroupLaunchPlanContract = { ...basePlan };
    delete planWithoutWorkflow.workflow;
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", ...Array.from({ length: 20 }, (_, index) => `lease-${index}`)),
      provisionAgent,
      createGroup: vi.fn().mockResolvedValue(groupResponse("group-1")),
      addGroupMembers: vi.fn().mockResolvedValue(groupResponse("group-1")),
      sendMessage: vi.fn().mockResolvedValue(messageResponse("message-1")),
      enableRoutingPolicy,
    });
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: planWithoutWorkflow }));
    let completed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    while (completed?.status !== "completed") {
      completed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    }

    expect(enableRoutingPolicy).toHaveBeenCalledTimes(1);
    expect(enableRoutingPolicy.mock.calls[0]?.[0]).not.toHaveProperty("coordinatorAgentId");
  });

  it("cancels before side effects without recording cleanup", async () => {
    const store = new MemoryStore();
    const runner = createGroupProvisioningRunner({ store, newId: ids("job-1", "lease-1", "lease-2") });
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: basePlan }));

    const canceled = await runner.cancelJob(created.id, { choice: "soft_delete_generated_agents", reason: "user changed course" });

    expect(canceled?.status).toBe("canceled");
    expect(canceled?.errorSummary).toBe("user changed course");
    expect(canceled?.stepResults).toEqual([]);
  });

  it("soft-deletes only generated agents on cancellation after side effects", async () => {
    const store = new MemoryStore();
    const softDisableGeneratedAgents = vi.fn().mockResolvedValue(undefined);
    const provisionAgent = vi.fn().mockResolvedValue(agentResponse("agent-backend", "backend-engineer"));
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", "lease-1", "lease-2", "lease-3", "lease-4"),
      provisionAgent,
      softDisableGeneratedAgents,
      loadExistingAgentCandidates: vi.fn().mockResolvedValue([existingAgentCandidate()]),
    });
    const plan = reusePlan();
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan }));
    await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 3 });

    const canceled = await runner.cancelJob(created.id, { choice: "soft_delete_generated_agents" });

    expect(softDisableGeneratedAgents).toHaveBeenCalledWith({
      actorId: "human-1",
      companyId: "company-1",
      agentIds: ["agent-backend"],
    });
    expect(canceled?.stepResults).toEqual(expect.arrayContaining([
      expect.objectContaining({
        kind: "cleanup",
        result: "soft_deleted_generated_agents",
        softDeletedAgentIds: ["agent-backend"],
        preservedAgentIds: ["agent-operator"],
      }),
      expect.objectContaining({
        kind: "residual_assets",
        generatedAgentIds: ["agent-backend"],
        reusedAgentIds: ["agent-operator"],
      }),
    ]));
  });

  it("soft-deletes unrecorded generated agents discovered by launch-plan name", async () => {
    const store = new MemoryStore();
    const softDisableGeneratedAgents = vi.fn().mockResolvedValue(undefined);
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", "lease-1", "lease-2"),
      softDisableGeneratedAgents,
      loadGeneratedAgentCleanupCandidates: vi.fn().mockResolvedValue([
        existingAgentCandidate({
          principal: {
            ...existingAgentCandidate().principal,
            id: "agent-orphan",
            name: "project-operator",
            created_at: "2026-05-19T00:00:01.000Z",
          },
        }),
        existingAgentCandidate({
          principal: {
            ...existingAgentCandidate().principal,
            id: "agent-preexisting",
            name: "backend-engineer",
            created_at: "2026-05-18T23:59:59.000Z",
          },
        }),
      ]),
    });
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: basePlan }));

    const canceled = await runner.cancelJob(created.id, { choice: "soft_delete_generated_agents" });

    expect(softDisableGeneratedAgents).toHaveBeenCalledWith({
      actorId: "human-1",
      companyId: "company-1",
      agentIds: ["agent-orphan"],
    });
    expect(canceled?.stepResults).toEqual(expect.arrayContaining([
      expect.objectContaining({
        kind: "cleanup",
        result: "soft_deleted_generated_agents",
        softDeletedAgentIds: ["agent-orphan"],
      }),
    ]));
  });

  it("records custom workspace paths as preserved residual assets during cleanup", async () => {
    const store = new MemoryStore();
    const softDisableGeneratedAgents = vi.fn().mockResolvedValue(undefined);
    const provisionAgent = vi.fn().mockResolvedValue(agentResponse("agent-operator", "project-operator"));
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", "lease-1", "lease-2", "lease-3"),
      provisionAgent,
      softDisableGeneratedAgents,
    });
    const plan: GroupLaunchPlanContract = {
      ...basePlan,
      rolePlans: [
        {
          slotId: "project-operator",
          action: "create",
          agentName: "project-operator",
          roleTemplateId: "project-operator",
          roleTemplateVersion: "1.0.0",
          driver: "codex_terminal",
          instructionStatus: "group_context_added",
          setupInputs: {},
          selectedSkills: [],
          workspaceMode: "custom",
          workspacePath: "/Users/alice/project",
        },
        ...basePlan.rolePlans.slice(1),
      ],
    };
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan }));
    await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 2 });

    const canceled = await runner.cancelJob(created.id, { choice: "soft_delete_generated_agents" });

    expect(softDisableGeneratedAgents).toHaveBeenCalledWith(expect.objectContaining({
      agentIds: ["agent-operator"],
    }));
    expect(canceled?.stepResults).toEqual(expect.arrayContaining([
      expect.objectContaining({
        kind: "residual_assets",
        customWorkspacePathsPreserved: ["/Users/alice/project"],
      }),
    ]));
  });

  it("blocks group creation after required-agent failure and offers cleanup recovery", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn().mockRejectedValueOnce(new Error("driver unavailable"));
    const createGroup = vi.fn();
    const runner = createGroupProvisioningRunner({ store, newId: ids("job-1", "lease-1", "lease-2"), provisionAgent, createGroup });
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: basePlan }));
    await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });

    const failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id });

    expect(failed?.status).toBe("partial_failure");
    expect(failed?.issues[0]).toMatchObject({ code: "required_agent_creation_failed", roleSlotId: "project-operator" });
    expect(failed?.recoveryChoices.map((choice) => choice.id)).toEqual([
      "retry_agent_creation",
      "edit_plan",
      "soft_delete_generated_agents",
      "cancel",
    ]);
    expect(createGroup).not.toHaveBeenCalled();
  });

  it("allows optional-agent failure to be skipped", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn()
      .mockResolvedValueOnce(agentResponse("agent-operator", "project-operator"))
      .mockResolvedValueOnce(agentResponse("agent-backend", "backend-engineer"))
      .mockResolvedValueOnce(agentResponse("agent-reviewer", "code-reviewer"))
      .mockRejectedValueOnce(new Error("optional driver unavailable"));
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", ...Array.from({ length: 12 }, (_, index) => `lease-${index}`)),
      provisionAgent,
    });
    const plan = {
      ...basePlan,
      rolePlans: [
        ...basePlan.rolePlans.slice(0, 3),
        {
          ...basePlan.rolePlans[3],
          action: "create" as const,
          agentName: "frontend-engineer",
          driver: "codex_terminal" as const,
          instructionStatus: "group_context_added" as const,
          setupInputs: {},
          selectedSkills: [],
          workspaceMode: "none" as const,
        },
      ],
    };
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan }));
    let failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    while (failed?.status !== "partial_failure") {
      failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    }

    const skipped = await runner.retryJob(created.id, { choice: "skip_optional_role", roleSlotId: "frontend-engineer" });

    expect(failed.issues[0]).toMatchObject({ code: "optional_agent_creation_failed", roleSlotId: "frontend-engineer" });
    expect(failed.recoveryChoices.map((choice) => choice.id)).toContain("skip_optional_role");
    expect(skipped?.status).toBe("creating_agents");
    expect(skipped?.stepResults).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: "skipped_optional_role", roleSlotId: "frontend-engineer", reason: "recovery_choice" }),
    ]));
  });

  it("soft-deletes generated agents after group creation failure recovery", async () => {
    const store = new MemoryStore();
    const softDisableGeneratedAgents = vi.fn().mockResolvedValue(undefined);
    const provisionAgent = vi.fn()
      .mockResolvedValueOnce(agentResponse("agent-operator", "project-operator"))
      .mockResolvedValueOnce(agentResponse("agent-backend", "backend-engineer"))
      .mockResolvedValueOnce(agentResponse("agent-reviewer", "code-reviewer"));
    const createGroup = vi.fn().mockRejectedValueOnce(new Error("group unavailable"));
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", ...Array.from({ length: 12 }, (_, index) => `lease-${index}`)),
      provisionAgent,
      createGroup,
      softDisableGeneratedAgents,
    });
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: basePlan }));
    let failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    while (failed?.status !== "failed") {
      failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    }

    const rolledBack = await runner.retryJob(created.id, { choice: "soft_delete_generated_agents" });

    expect(failed.recoveryChoices.map((choice) => choice.id)).toEqual(["retry_group_creation", "soft_delete_generated_agents", "cancel"]);
    expect(softDisableGeneratedAgents).toHaveBeenCalledWith(expect.objectContaining({
      agentIds: ["agent-operator", "agent-backend", "agent-reviewer"],
    }));
    expect(rolledBack?.status).toBe("rolled_back");
    expect(rolledBack?.stepResults).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: "cleanup", result: "soft_deleted_generated_agents" }),
      expect.objectContaining({ kind: "residual_assets", generatedAgentIds: ["agent-operator", "agent-backend", "agent-reviewer"] }),
    ]));
  });

  it("keeps group and agents after member-add failure and can skip optional member", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn()
      .mockResolvedValueOnce(agentResponse("agent-operator", "project-operator"))
      .mockResolvedValueOnce(agentResponse("agent-backend", "backend-engineer"))
      .mockResolvedValueOnce(agentResponse("agent-reviewer", "code-reviewer"))
      .mockResolvedValueOnce(agentResponse("agent-frontend", "frontend-engineer"));
    const addGroupMembers = vi.fn()
      .mockResolvedValueOnce(groupResponse("group-1"))
      .mockResolvedValueOnce(groupResponse("group-1"))
      .mockResolvedValueOnce(groupResponse("group-1"))
      .mockRejectedValueOnce(new Error("invite failed"));
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", ...Array.from({ length: 20 }, (_, index) => `lease-${index}`)),
      provisionAgent,
      createGroup: vi.fn().mockResolvedValue(groupResponse("group-1")),
      addGroupMembers,
      enableRoutingPolicy: vi.fn().mockResolvedValue(undefined),
      sendMessage: vi.fn().mockResolvedValue(messageResponse("msg-1")),
    });
    const plan = {
      ...basePlan,
      rolePlans: [
        ...basePlan.rolePlans.slice(0, 3),
        {
          ...basePlan.rolePlans[3],
          action: "create" as const,
          agentName: "frontend-engineer",
          driver: "codex_terminal" as const,
          instructionStatus: "group_context_added" as const,
          setupInputs: {},
          selectedSkills: [],
          workspaceMode: "none" as const,
        },
      ],
    };
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan }));
    let failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    while (failed?.status !== "partial_failure") {
      failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    }

    const skipped = await runner.retryJob(created.id, { choice: "skip_optional_role", roleSlotId: "frontend-engineer" });

    expect(failed.createdGroupId).toBe("group-1");
    expect(failed.issues[0]).toMatchObject({ code: "optional_member_add_failed", roleSlotId: "frontend-engineer" });
    expect(failed.stepResults).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: "member_add", roleSlotId: "frontend-engineer", result: "failed" }),
      expect.objectContaining({ kind: "residual_assets", groupConversationId: "group-1" }),
    ]));
    expect(failed.recoveryChoices.map((choice) => choice.id)).toEqual([
      "retry_member_add",
      "replace_agent",
      "skip_optional_role",
      "manual_invite",
      "cancel",
    ]);
    expect(skipped?.status).toBe("adding_members");
    expect(skipped?.stepResults).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: "member_add", roleSlotId: "frontend-engineer", result: "skipped" }),
    ]));

    let completed = skipped;
    for (let attempt = 0; completed?.status !== "completed" && attempt < 10; attempt += 1) {
      if (completed && ["failed", "failed_validation", "partial_failure", "rolled_back", "canceled"].includes(completed.status)) break;
      completed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    }
    expect(completed?.status).toBe("completed");

    const persistedWorkflow = (store.groupTemplateInstances[0] as {
      workflow: { participantRoleDefaults: Record<string, string[]> };
    }).workflow;
    expect(persistedWorkflow.participantRoleDefaults).toEqual({
      "project-operator": ["coordinator"],
      "backend-engineer": ["owner"],
      "code-reviewer": ["quality_check"],
    });
    expect(persistedWorkflow.participantRoleDefaults).not.toHaveProperty("frontend-engineer");
    expect(store.roleAssignments).toEqual(expect.arrayContaining([
      expect.objectContaining({
        id: "group-provisioning:job-1:idem-1:assignment:frontend-engineer",
        slotId: "frontend-engineer",
        action: "skipped",
      }),
    ]));
  });

  it("ignores stale role-scoped recovery requests for a different slot", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn()
      .mockResolvedValueOnce(agentResponse("agent-operator", "project-operator"))
      .mockResolvedValueOnce(agentResponse("agent-backend", "backend-engineer"))
      .mockResolvedValueOnce(agentResponse("agent-reviewer", "code-reviewer"))
      .mockResolvedValueOnce(agentResponse("agent-frontend", "frontend-engineer"));
    const addGroupMembers = vi.fn()
      .mockResolvedValueOnce(groupResponse("group-1"))
      .mockResolvedValueOnce(groupResponse("group-1"))
      .mockResolvedValueOnce(groupResponse("group-1"))
      .mockRejectedValueOnce(new Error("invite failed"));
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", ...Array.from({ length: 20 }, (_, index) => `lease-${index}`)),
      provisionAgent,
      createGroup: vi.fn().mockResolvedValue(groupResponse("group-1")),
      addGroupMembers,
    });
    const plan = {
      ...basePlan,
      rolePlans: [
        ...basePlan.rolePlans.slice(0, 3),
        {
          ...basePlan.rolePlans[3],
          action: "create" as const,
          agentName: "frontend-engineer",
          driver: "codex_terminal" as const,
          instructionStatus: "group_context_added" as const,
          setupInputs: {},
          selectedSkills: [],
          workspaceMode: "none" as const,
        },
      ],
    };
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan }));
    let failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    while (failed?.status !== "partial_failure") {
      failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    }

    const ignored = await runner.retryJob(created.id, { choice: "skip_optional_role", roleSlotId: "backend-engineer" });

    expect(failed.issues[0]).toMatchObject({ code: "optional_member_add_failed", roleSlotId: "frontend-engineer" });
    expect(ignored?.status).toBe("partial_failure");
    expect(ignored?.stepResults.some((result) =>
      result.kind === "member_add" && result.roleSlotId === "backend-engineer" && result.result === "skipped"
    )).toBe(false);
  });

  it("keeps required member-add failures for manual invite without soft-deleting agents", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn()
      .mockResolvedValueOnce(agentResponse("agent-operator", "project-operator"))
      .mockResolvedValueOnce(agentResponse("agent-backend", "backend-engineer"))
      .mockResolvedValueOnce(agentResponse("agent-reviewer", "code-reviewer"));
    const addGroupMembers = vi.fn().mockRejectedValueOnce(new Error("invite failed"));
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", ...Array.from({ length: 16 }, (_, index) => `lease-${index}`)),
      provisionAgent,
      createGroup: vi.fn().mockResolvedValue(groupResponse("group-1")),
      addGroupMembers,
    });
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: basePlan }));
    let failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    while (failed?.status !== "partial_failure") {
      failed = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    }

    const manual = await runner.retryJob(created.id, { choice: "manual_invite", roleSlotId: "project-operator" });

    expect(failed.issues[0]).toMatchObject({ code: "required_member_add_failed", roleSlotId: "project-operator" });
    expect(failed.recoveryChoices.map((choice) => choice.id)).toEqual([
      "retry_member_add",
      "replace_agent",
      "manual_invite",
      "cancel",
    ]);
    expect(manual?.status).toBe("adding_members");
    expect(manual?.stepResults).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: "member_add", roleSlotId: "project-operator", result: "skipped" }),
    ]));
    expect(manual?.progressSteps).toEqual(expect.arrayContaining([
      expect.objectContaining({ id: "member:add:project-operator", status: "skipped" }),
    ]));
  });

  it("marks kickoff failure as completed with warning and allows kickoff retry or entering group", async () => {
    const store = new MemoryStore();
    const provisionAgent = vi.fn()
      .mockResolvedValueOnce(agentResponse("agent-operator", "project-operator"))
      .mockResolvedValueOnce(agentResponse("agent-backend", "backend-engineer"))
      .mockResolvedValueOnce(agentResponse("agent-reviewer", "code-reviewer"));
    const sendMessage = vi.fn()
      .mockRejectedValueOnce(new Error("message send failed"))
      .mockResolvedValueOnce(messageResponse("message-retry"));
    const runner = createGroupProvisioningRunner({
      store,
      newId: ids("job-1", ...Array.from({ length: 20 }, (_, index) => `lease-${index}`)),
      provisionAgent,
      createGroup: vi.fn().mockResolvedValue(groupResponse("group-1")),
      addGroupMembers: vi.fn().mockResolvedValue(groupResponse("group-1")),
      enableRoutingPolicy: vi.fn().mockResolvedValue(undefined),
      sendMessage,
    });
    const created = await runner.createJob(createInput({ idempotencyKey: "idem-1", plan: basePlan }));
    let warned = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    while (warned?.status !== "completed_with_warning") {
      warned = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 10 });
    }

    expect(warned.status).toBe("completed_with_warning");
    expect(warned.recoveryChoices.map((choice) => choice.id)).toEqual(["retry_kickoff", "enter_group"]);
    expect(warned.stepResults).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: "kickoff_post", result: "warning" }),
      expect.objectContaining({ kind: "residual_assets", groupConversationId: "group-1" }),
    ]));

    await runner.retryJob(created.id, { choice: "retry_kickoff" });
    const retried = await runner.runJob({ sessionToken: "token", actorId: "human-1", jobId: created.id, maxSteps: 2 });

    expect(retried?.status).toBe("completed");
    expect(retried?.progressSteps).toEqual(expect.arrayContaining([
      expect.objectContaining({ id: "kickoff:post", status: "succeeded" }),
    ]));
  });
});

function createInput(body: Omit<ProvisioningJobCreationRequest, "companyId"> & { companyId?: string }) {
  return {
    sessionToken: "token",
    actorId: "human-1",
    companyId: "company-1",
    body: {
      companyId: "company-1",
      ...body,
    },
  };
}

function reusePlan(): GroupLaunchPlanContract {
  return {
    ...basePlan,
    rolePlans: [
      {
        slotId: "project-operator",
        action: "reuse",
        existingAgentId: "agent-operator",
        roleTemplateId: "project-operator",
        roleTemplateVersion: "1.0.0",
      },
      ...basePlan.rolePlans.slice(1),
    ],
  };
}

function existingAgentCandidate(overrides: Partial<ExistingAgentCandidate> = {}): ExistingAgentCandidate {
  return {
    principal: {
      id: "agent-operator",
      workspace_id: "company-1",
      principal_type: "agent",
      name: "project-operator",
      avatar_url: null,
      scopes: [],
      disabled: false,
      created_at: "2026-05-19T00:00:00.000Z",
      updated_at: "2026-05-19T00:00:00.000Z",
    },
    runtimeBinding: {
      id: "binding-agent-operator",
      workspace_id: "company-1",
      conversation_id: "direct-agent-operator",
      conversation_type: "direct",
      agent_principal_id: "agent-operator",
      driver_type: "codex_terminal",
      workspace_path: "/tmp/workspace",
      state: "idle",
    },
    companyId: "company-1",
    workspaceId: "company-1",
    ...overrides,
  };
}

function ids(...values: string[]) {
  let index = 0;
  return () => values[index++] ?? `id-${index}`;
}

function dates(...values: string[]) {
  let index = 0;
  return () => new Date(values[index++] ?? values.at(-1) ?? "2026-05-19T00:00:00.000Z");
}

function agentResponse(agentId: string, name: string) {
  return {
    agent: {
      id: agentId,
      workspace_id: "company-1",
      principal_type: "agent" as const,
      name,
      avatar_url: null,
      scopes: [],
      disabled: false,
      created_at: "2026-05-19T00:00:00.000Z",
      updated_at: "2026-05-19T00:00:00.000Z",
    },
    secret: "",
    conversation: groupResponse(`direct-${agentId}`),
    binding: {
      id: `binding-${agentId}`,
      workspace_id: "company-1",
      conversation_id: `direct-${agentId}`,
      conversation_name: "Direct",
      conversation_type: "direct" as const,
      agent_principal_id: agentId,
      agent_name: name,
      driver_type: "codex_terminal" as const,
      workspace_path: "/tmp/workspace",
      git_worktree_path: null,
      external_session_id: null,
      external_thread_id: null,
      last_event_cursor: 0,
      last_acked_event_cursor: 0,
      last_seen_server_seq: 0,
      state: "idle" as const,
      last_error: null,
      updated_at: "2026-05-19T00:00:00.000Z",
    },
    workspace_path: "/tmp/workspace",
  };
}

function groupResponse(id: string) {
  return {
    id,
    workspace_id: "company-1",
    conversation_type: "group" as const,
    name: "onboarding-mvp",
    description: null,
    avatar_url: null,
    creator_id: "human-1",
    created_at: "2026-05-19T00:00:00.000Z",
    updated_at: "2026-05-19T00:00:00.000Z",
    members: {},
  };
}

function messageResponse(id: string) {
  return {
    id,
    workspace_id: "company-1",
    conversation_id: "group-1",
    sender_id: "human-1",
    content: basePlan.kickoffText,
    content_type: "text",
    metadata: {},
    edited_at: null,
    edited_by: null,
    server_seq: 1,
    idempotency_key: "kickoff",
    created_at: "2026-05-19T00:00:00.000Z",
  };
}

class MemoryStore implements GroupProvisioningStore {
  jobs: GroupProvisioningJob[] = [];
  agentTemplateInstances: unknown[] = [];
  groupTemplateInstances: unknown[] = [];
  roleAssignments: unknown[] = [];

  async createJob(input: CreateGroupProvisioningJobInput): Promise<GroupProvisioningJob> {
    return this.insertJob(input);
  }

  async createJobByIdempotencyKey(input: CreateGroupProvisioningJobInput): Promise<GroupProvisioningJob> {
    const existing = this.jobs.find((job) => job.companyId === input.companyId && job.idempotencyKey === input.idempotencyKey);
    return existing ?? this.insertJob(input);
  }

  async getJob(jobId: string): Promise<GroupProvisioningJob | null> {
    return this.jobs.find((job) => job.id === jobId) ?? null;
  }

  async getJobByIdempotencyKey(companyId: string, idempotencyKey: string): Promise<GroupProvisioningJob | null> {
    return this.jobs.find((job) => job.companyId === companyId && job.idempotencyKey === idempotencyKey) ?? null;
  }

  async listActiveJobsForAgentIds(): Promise<GroupProvisioningJob[]> {
    return [];
  }

  async acquireLease(input: {
    jobId: string;
    leaseOwner: string;
    leaseToken: string;
    leaseMs: number;
    now?: Date;
  }): Promise<GroupProvisioningJob | null> {
    const job = await this.getJob(input.jobId);
    const currentNow = input.now ?? new Date();
    if (!job || ["completed", "rolled_back", "canceled"].includes(job.status)) return null;
    if (job.leaseToken && job.leaseExpiresAt && Date.parse(String(job.leaseExpiresAt)) > currentNow.getTime()) return null;
    job.leaseOwner = input.leaseOwner;
    job.leaseToken = input.leaseToken;
    job.leaseExpiresAt = new Date(currentNow.getTime() + input.leaseMs).toISOString();
    job.updatedAt = currentNow.toISOString();
    return job;
  }

  async releaseLease(input: { jobId: string; leaseToken: string; now?: Date }): Promise<GroupProvisioningJob | null> {
    const job = await this.getJob(input.jobId);
    if (!job || job.leaseToken !== input.leaseToken) return null;
    job.leaseOwner = null;
    job.leaseToken = null;
    job.leaseExpiresAt = null;
    job.updatedAt = (input.now ?? new Date()).toISOString();
    return job;
  }

  async releaseStaleLeases(input: { now?: Date; jobId?: string } = {}): Promise<number> {
    const currentNow = input.now ?? new Date();
    let count = 0;
    for (const job of this.jobs) {
      if (input.jobId && job.id !== input.jobId) continue;
      if (job.leaseToken && job.leaseExpiresAt && Date.parse(String(job.leaseExpiresAt)) <= currentNow.getTime()) {
        job.leaseOwner = null;
        job.leaseToken = null;
        job.leaseExpiresAt = null;
        count += 1;
      }
    }
    return count;
  }

  async updateJobAfterLease(input: UpdateGroupProvisioningJobInput): Promise<GroupProvisioningJob> {
    const job = await this.getJob(input.jobId);
    const currentNow = input.now ?? new Date();
    if (!job || job.leaseToken !== input.leaseToken) throw new Error("updateJobAfterLease did not return a group provisioning job");
    if (!job.leaseExpiresAt || Date.parse(String(job.leaseExpiresAt)) <= currentNow.getTime()) {
      throw new Error("updateJobAfterLease did not return a group provisioning job");
    }
    if (input.status) job.status = input.status;
    if (input.planJson !== undefined) {
      job.planJson = sanitizeProvisioningJson(input.planJson, "planJson");
    }
    if (input.stepResultsJson !== undefined) {
      job.stepResultsJson = sanitizeProvisioningJson(input.stepResultsJson, "stepResultsJson");
    }
    if (input.involvedAgentIds) job.involvedAgentIds = input.involvedAgentIds;
    if (input.createdAgentIds) job.createdAgentIds = input.createdAgentIds;
    if (input.createdGroupId !== undefined && input.createdGroupId !== null) job.createdGroupId = input.createdGroupId;
    if (input.errorSummary !== undefined) job.errorSummary = input.errorSummary;
    if (input.status === "completed" || input.status === "completed_with_warning") job.completedAt = currentNow.toISOString();
    job.updatedAt = currentNow.toISOString();
    return job;
  }

  async cancelJob(input: { jobId: string; leaseToken?: string; errorSummary?: string | null; now?: Date }): Promise<GroupProvisioningJob> {
    const job = await this.getJob(input.jobId);
    if (!job) throw new Error("cancelJob did not return a group provisioning job");
    if (input.leaseToken && job.leaseToken !== input.leaseToken) throw new Error("cancelJob did not return a group provisioning job");
    job.status = "canceled";
    job.errorSummary = input.errorSummary ?? job.errorSummary;
    job.canceledAt = (input.now ?? new Date()).toISOString();
    job.leaseOwner = null;
    job.leaseToken = null;
    job.leaseExpiresAt = null;
    return job;
  }

  async prepareRetry(input: { jobId: string; nextStatus: GroupProvisioningJobStatus; now?: Date }): Promise<GroupProvisioningJob> {
    const job = await this.getJob(input.jobId);
    if (!job) throw new Error("prepareRetry did not return a group provisioning job");
    job.status = input.nextStatus;
    job.errorSummary = null;
    const state = this.state(job.stepResultsJson);
    job.stepResultsJson = { results: state.results, issues: [], agentSteps: state.agentSteps } as JsonValue;
    job.updatedAt = (input.now ?? new Date()).toISOString();
    return job;
  }

  async insertAgentTemplateInstance(input: unknown): Promise<void> {
    this.agentTemplateInstances.push(input);
  }

  async insertGroupTemplateInstance(input: unknown): Promise<void> {
    const row = input as Record<string, unknown>;
    const existingIndex = this.groupTemplateInstances.findIndex(
      (instance) => (instance as Record<string, unknown>).groupConversationId === row.groupConversationId,
    );
    if (existingIndex >= 0) this.groupTemplateInstances[existingIndex] = row;
    else this.groupTemplateInstances.push(row);
  }

  async insertRoleAssignment(input: unknown): Promise<void> {
    const row = input as Record<string, unknown>;
    const existingIndex = this.roleAssignments.findIndex(
      (assignment) => (assignment as Record<string, unknown>).id === row.id,
    );
    if (existingIndex >= 0) this.roleAssignments[existingIndex] = row;
    else this.roleAssignments.push(row);
  }

  private insertJob(input: CreateGroupProvisioningJobInput): GroupProvisioningJob {
    const now = "2026-05-19T00:00:00.000Z";
    const job: GroupProvisioningJob = {
      id: input.id,
      companyId: input.companyId,
      requestedBy: input.requestedBy,
      groupTemplateId: input.groupTemplateId,
      groupTemplateVersion: input.groupTemplateVersion,
      status: input.initialStatus ?? "validating",
      idempotencyKey: input.idempotencyKey,
      planJson: input.planJson,
      stepResultsJson: {},
      involvedAgentIds: input.involvedAgentIds ?? [],
      createdAgentIds: [],
      createdGroupId: null,
      errorSummary: null,
      leaseOwner: null,
      leaseToken: null,
      leaseExpiresAt: null,
      createdAt: now,
      updatedAt: now,
      completedAt: null,
      canceledAt: null,
    };
    this.jobs.push(job);
    return job;
  }

  private results(value: JsonValue): JsonValue[] {
    return this.state(value).results;
  }

  private state(value: JsonValue): { results: JsonValue[]; agentSteps: Record<string, unknown> } {
    if (Array.isArray(value)) return { results: value, agentSteps: {} };
    if (value && typeof value === "object" && !Array.isArray(value)) {
      return {
        results: Array.isArray(value.results) ? value.results as JsonValue[] : [],
        agentSteps: value.agentSteps && typeof value.agentSteps === "object" && !Array.isArray(value.agentSteps)
          ? value.agentSteps as Record<string, unknown>
          : {},
      };
    }
    return { results: [], agentSteps: {} };
  }
}
