import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import {
  GROUP_PROVISIONING_JOB_STATUSES,
  assertGroupProvisioningStatusTransition,
  createGroupProvisioningStore,
  sanitizeProvisioningJson,
  type JsonValue,
  type QueryClient,
  type QueryStatement,
} from "./group-provisioning-store";

describe("group provisioning store", () => {
  it("creates jobs idempotently by company id and idempotency key", async () => {
    const db = new FakeQueryClient();
    const store = createGroupProvisioningStore(db);

    const first = await store.createJobByIdempotencyKey(jobInput("job-1", "idem-1"));
    const second = await store.createJobByIdempotencyKey(jobInput("job-2", "idem-1"));

    expect(first.id).toBe("job-1");
    expect(second.id).toBe("job-1");
    expect(db.jobs).toHaveLength(1);
  });

  it("acquires leases only when there is no active lease or the lease is stale", async () => {
    const db = new FakeQueryClient();
    const store = createGroupProvisioningStore(db);
    await store.createJob(jobInput("job-1", "idem-1"));

    const now = new Date("2026-05-19T00:00:00.000Z");
    const first = await store.acquireLease({
      jobId: "job-1",
      leaseOwner: "worker-a",
      leaseToken: "lease-a",
      leaseMs: 60_000,
      now,
    });
    const blocked = await store.acquireLease({
      jobId: "job-1",
      leaseOwner: "worker-b",
      leaseToken: "lease-b",
      leaseMs: 60_000,
      now: new Date("2026-05-19T00:00:10.000Z"),
    });
    const stale = await store.acquireLease({
      jobId: "job-1",
      leaseOwner: "worker-b",
      leaseToken: "lease-b",
      leaseMs: 60_000,
      now: new Date("2026-05-19T00:01:01.000Z"),
    });

    expect(first?.leaseOwner).toBe("worker-a");
    expect(blocked).toBeNull();
    expect(stale?.leaseOwner).toBe("worker-b");
    expect(stale?.leaseToken).toBe("lease-b");
  });

  it("releases stale leases without touching live leases", async () => {
    const db = new FakeQueryClient();
    const store = createGroupProvisioningStore(db);
    await store.createJob(jobInput("job-1", "idem-1"));
    await store.createJob(jobInput("job-2", "idem-2"));
    await store.acquireLease({
      jobId: "job-1",
      leaseOwner: "worker-a",
      leaseToken: "lease-a",
      leaseMs: 1_000,
      now: new Date("2026-05-19T00:00:00.000Z"),
    });
    await store.acquireLease({
      jobId: "job-2",
      leaseOwner: "worker-b",
      leaseToken: "lease-b",
      leaseMs: 60_000,
      now: new Date("2026-05-19T00:00:00.000Z"),
    });

    const released = await store.releaseStaleLeases({ now: new Date("2026-05-19T00:00:02.000Z") });

    expect(released).toBe(1);
    expect((await store.getJob("job-1"))?.leaseToken).toBeNull();
    expect((await store.getJob("job-2"))?.leaseToken).toBe("lease-b");
  });

  it("validates canonical statuses and transitions before updating leased jobs", async () => {
    const db = new FakeQueryClient();
    const store = createGroupProvisioningStore(db);
    await store.createJob(jobInput("job-1", "idem-1"));

    expect(GROUP_PROVISIONING_JOB_STATUSES).toContain("partial_failure");
    expect(() => assertGroupProvisioningStatusTransition("validating", "completed")).toThrow(
      /invalid group provisioning status transition/,
    );
    expect(() => assertGroupProvisioningStatusTransition("failed", "rolled_back")).not.toThrow();
    expect(() => assertGroupProvisioningStatusTransition("partial_failure", "validating")).not.toThrow();
    expect(() => assertGroupProvisioningStatusTransition("completed_with_warning", "posting_kickoff")).not.toThrow();

    await expect(
      store.updateJobAfterLease({
        jobId: "job-1",
        leaseToken: "lease-a",
        status: "completed",
      }),
    ).rejects.toThrow(/invalid group provisioning status transition/);

    await store.acquireLease({
      jobId: "job-1",
      leaseOwner: "worker-a",
      leaseToken: "lease-a",
      leaseMs: 60_000,
      now: new Date("2026-05-19T00:00:00.000Z"),
    });
    const next = await store.updateJobAfterLease({
      jobId: "job-1",
      leaseToken: "lease-a",
      status: "creating_agents",
      stepResultsJson: { validation: { ok: true } },
      now: new Date("2026-05-19T00:00:01.000Z"),
    });

    expect(next.status).toBe("creating_agents");
    expect(next.stepResultsJson).toEqual({ validation: { ok: true } });
  });

  it("requires matching live leases for runner updates and blocks unlocked cancel during active leases", async () => {
    const db = new FakeQueryClient();
    const store = createGroupProvisioningStore(db);
    await store.createJob(jobInput("job-1", "idem-1"));
    await store.acquireLease({
      jobId: "job-1",
      leaseOwner: "worker-a",
      leaseToken: "lease-a",
      leaseMs: 60_000,
      now: new Date("2026-05-19T00:00:00.000Z"),
    });

    await expect(
      store.updateJobAfterLease({
        jobId: "job-1",
        leaseToken: "lease-b",
        status: "creating_agents",
        now: new Date("2026-05-19T00:00:01.000Z"),
      }),
    ).rejects.toThrow(/updateJobAfterLease did not return/);
    await expect(
      store.cancelJob({
        jobId: "job-1",
        errorSummary: "User clicked cancel while worker still owns the lease.",
        now: new Date("2026-05-19T00:00:01.000Z"),
      }),
    ).rejects.toThrow(/cancelJob did not return/);

    const canceled = await store.cancelJob({
      jobId: "job-1",
      leaseToken: "lease-a",
      errorSummary: "Worker-aware cancel.",
      now: new Date("2026-05-19T00:00:02.000Z"),
    });

    expect(canceled.status).toBe("canceled");
    expect(canceled.leaseToken).toBeNull();
  });

  it("lists active jobs that involve conflicting agent ids", async () => {
    const db = new FakeQueryClient();
    const store = createGroupProvisioningStore(db);
    await store.createJob({
      ...jobInput("job-1", "idem-1"),
      involvedAgentIds: ["agent-a"],
    });
    await store.createJob({
      ...jobInput("job-2", "idem-2"),
      initialStatus: "completed",
      involvedAgentIds: ["agent-a"],
    });
    await store.createJob(jobInput("job-3", "idem-3"));
    await store.acquireLease({
      jobId: "job-3",
      leaseOwner: "worker-a",
      leaseToken: "lease-a",
      leaseMs: 60_000,
      now: new Date("2026-05-19T00:00:00.000Z"),
    });
    await store.updateJobAfterLease({
      jobId: "job-3",
      leaseToken: "lease-a",
      status: "creating_agents",
      createdAgentIds: ["agent-b"],
      now: new Date("2026-05-19T00:00:01.000Z"),
    });

    await expect(store.listActiveJobsForAgentIds({ companyId: "company-1", agentIds: [] })).resolves.toEqual([]);
    expect((await store.listActiveJobsForAgentIds({ companyId: "company-1", agentIds: ["agent-a"] })).map((job) => job.id)).toEqual(["job-1"]);
    expect((await store.listActiveJobsForAgentIds({ companyId: "company-1", agentIds: ["agent-b"] })).map((job) => job.id)).toEqual(["job-3"]);
  });

  it("records cancellation timestamps when jobs are canceled", async () => {
    const db = new FakeQueryClient();
    const store = createGroupProvisioningStore(db);
    await store.createJob(jobInput("job-1", "idem-1"));

    const canceled = await store.cancelJob({
      jobId: "job-1",
      errorSummary: "User canceled before side effects.",
    });

    expect(canceled.status).toBe("canceled");
    expect(canceled.canceledAt).toBeTruthy();
    expect(canceled.errorSummary).toBe("User canceled before side effects.");
  });

  it("inserts provenance rows with template metadata and role assignment shapes", async () => {
    const db = new FakeQueryClient();
    const store = createGroupProvisioningStore(db);

    await store.insertAgentTemplateInstance({
      agentPrincipalId: "agent-1",
      roleTemplateId: "backend-engineer",
      roleTemplateVersion: "1.0.0",
      instructionStatus: "group_context_added",
      setupSummary: { repository_path: "/repo" },
      workspaceMode: "custom",
      selectedSkills: ["repo-navigation"],
      originatingJobId: "job-1",
    });
    await store.insertGroupTemplateInstance({
      groupConversationId: "conv-1",
      groupTemplateId: "software-development-team",
      groupTemplateVersion: "1.0.0",
      mission: "Ship onboarding",
      workflow: { steps: ["Build", "Review"], description: "Build -> Review" },
      kickoffText: "Mission: Ship onboarding\n\nRoles: Project Operator.\n\nNext user action: send the first concrete work item or question when ready.",
      outputContract: {
        summary: "Working feature",
        format: "Implementation report",
        requiredSections: ["Changes", "Tests"],
      },
      originatingJobId: "job-1",
    });
    await store.insertRoleAssignment({
      id: "assignment-1",
      groupConversationId: "conv-1",
      slotId: "backend",
      required: true,
      action: "created",
      agentPrincipalId: "agent-1",
      roleTemplateId: "backend-engineer",
      roleTemplateVersion: "1.0.0",
      instructionStatus: "group_context_added",
      originatingJobId: "job-1",
    });

    expect(db.agentTemplateInstances[0]).toMatchObject({
      agent_principal_id: "agent-1",
      role_template_id: "backend-engineer",
      instruction_status: "group_context_added",
      workspace_mode: "custom",
      selected_skills: ["repo-navigation"],
    });
    expect(db.groupTemplateInstances[0]).toMatchObject({
      group_conversation_id: "conv-1",
      group_template_id: "software-development-team",
      mission: "Ship onboarding",
    });
    expect(db.roleAssignments[0]).toMatchObject({
      action: "created",
      agent_principal_id: "agent-1",
      slot_id: "backend",
    });
    const sqlByName = Object.fromEntries(db.queries.map((query) => [query.name, query.text]));
    expect(sqlByName["agent-template-instance-insert"]).not.toContain("group_conversation_id");
    expect(sqlByName["group-template-instance-insert"]).toContain("ON CONFLICT (group_conversation_id)");
    expect(sqlByName["group-template-role-assignment-insert"]).toContain("ON CONFLICT (id)");
  });

  it("rejects sensitive keys recursively before storing plans or step results", async () => {
    const db = new FakeQueryClient();
    const store = createGroupProvisioningStore(db);

    expect(() =>
      sanitizeProvisioningJson({
        roles: [{ webhookSigningSecret: "nope" }],
      }),
    ).toThrow(/sensitive provisioning key/);

    await expect(
      store.createJob({
        ...jobInput("job-1", "idem-1"),
        planJson: { setup: { bearer_token: "nope" } },
      }),
    ).rejects.toThrow(/sensitive provisioning key/);
    expect(db.jobs).toHaveLength(0);
  });

  it("rejects skipped role assignments with an agent and created assignments without one", async () => {
    const store = createGroupProvisioningStore(new FakeQueryClient());

    await expect(
      store.insertRoleAssignment({
        id: "assignment-1",
        groupConversationId: "conv-1",
        slotId: "optional",
        required: false,
        action: "skipped",
        agentPrincipalId: "agent-1",
        roleTemplateId: "backend-engineer",
        roleTemplateVersion: "1.0.0",
      } as never),
    ).rejects.toThrow(/skipped role assignments cannot reference an agent/);

    await expect(
      store.insertRoleAssignment({
        id: "assignment-2",
        groupConversationId: "conv-1",
        slotId: "backend",
        required: true,
        action: "created",
        roleTemplateId: "backend-engineer",
        roleTemplateVersion: "1.0.0",
      } as never),
    ).rejects.toThrow(/created role assignments must reference an agent/);

    await expect(
      store.insertRoleAssignment({
        id: "assignment-3",
        groupConversationId: "conv-1",
        slotId: "reviewer",
        required: true,
        action: "created",
        agentPrincipalId: "agent-1",
        roleTemplateId: "code-reviewer",
        roleTemplateVersion: "1.0.0",
      } as never),
    ).rejects.toThrow(/created role assignments must record instruction status/);
  });
});

describe("group provisioning migration shape", () => {
  const migrationSql = readFileSync("../../migrations/V017__onboarding_template_provisioning.sql", "utf8");

  it("creates durable job and provenance tables outside runtime binding config", () => {
    expect(migrationSql).toContain("CREATE TABLE IF NOT EXISTS group_provisioning_job");
    expect(migrationSql).toContain("CREATE TABLE IF NOT EXISTS agent_template_instance");
    expect(migrationSql).toContain("CREATE TABLE IF NOT EXISTS group_template_instance");
    expect(migrationSql).toContain("CREATE TABLE IF NOT EXISTS group_template_role_assignment");
    expect(migrationSql).not.toMatch(/\b(ALTER|UPDATE|INSERT INTO)\s+agent_runtime_bindings\b/i);
  });

  it("declares canonical statuses, safe JSON defaults, provenance FKs, and operational indexes", () => {
    for (const status of GROUP_PROVISIONING_JOB_STATUSES) {
      expect(migrationSql).toContain(`'${status}'`);
    }
    expect(migrationSql).toContain("UNIQUE (company_id, idempotency_key)");
    expect(migrationSql).toContain("plan_json JSONB NOT NULL DEFAULT '{}'::jsonb");
    expect(migrationSql).toContain("step_results_json JSONB NOT NULL DEFAULT '{}'::jsonb");
    expect(migrationSql).toContain("involved_agent_ids TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[]");
    expect(migrationSql).toContain("created_agent_ids TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[]");
    expect(migrationSql).toContain("REFERENCES company(id)");
    expect(migrationSql).toContain("REFERENCES principal(id)");
    expect(migrationSql).toContain("REFERENCES conversation(id)");
    expect(migrationSql).toContain("group_provisioning_job_active_company_idx");
    expect(migrationSql).toContain("group_provisioning_job_active_involved_agents_idx");
    expect(migrationSql).toContain("group_provisioning_job_active_created_agents_idx");
    expect(migrationSql).toContain("group_provisioning_job_lease_idx");
    expect(migrationSql).toContain("group_template_role_assignment_slot_idx");
    expect(migrationSql).toContain("instruction_status IS NOT NULL");
  });
});

function jobInput(id: string, idempotencyKey: string) {
  return {
    id,
    companyId: "company-1",
    requestedBy: "human-1",
    groupTemplateId: "software-development-team",
    groupTemplateVersion: "1.0.0" as const,
    idempotencyKey,
    planJson: { mission: "Ship onboarding" },
  };
}

type JobRow = {
  id: string;
  company_id: string;
  requested_by: string;
  group_template_id: string;
  group_template_version: "1.0.0";
  status: string;
  idempotency_key: string;
  plan_json: JsonValue;
  step_results_json: JsonValue;
  involved_agent_ids: string[];
  created_agent_ids: string[];
  created_group_id: string | null;
  error_summary: string | null;
  lease_owner: string | null;
  lease_token: string | null;
  lease_expires_at: string | null;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
  canceled_at: string | null;
};

class FakeQueryClient implements QueryClient {
  jobs: JobRow[] = [];
  agentTemplateInstances: Record<string, unknown>[] = [];
  groupTemplateInstances: Record<string, unknown>[] = [];
  roleAssignments: Record<string, unknown>[] = [];
  queries: Array<{ name: string; text: string }> = [];

  async query<T extends object = Record<string, unknown>>(
    statement: QueryStatement,
    values: readonly unknown[] = [],
  ): Promise<{ rows: T[]; rowCount?: number | null }> {
    const name = typeof statement === "string" ? statement : (statement.name ?? statement.text);
    const queryText = typeof statement === "string" ? statement : statement.text;
    this.queries.push({ name, text: queryText });

    switch (name) {
      case "group-provisioning-job-create":
        return { rows: [this.insertJob(values)] as T[] };
      case "group-provisioning-job-create-idempotent": {
        const existing = this.jobs.find(
          (job) => job.company_id === values[1] && job.idempotency_key === values[6],
        );
        return { rows: existing ? [] : ([this.insertJob(values)] as T[]) };
      }
      case "group-provisioning-job-get":
        return { rows: this.jobs.filter((job) => job.id === values[0]) as T[] };
      case "group-provisioning-job-get-by-idempotency-key":
        return {
          rows: this.jobs.filter((job) => job.company_id === values[0] && job.idempotency_key === values[1]) as T[],
        };
      case "group-provisioning-job-list-active-agent-conflicts":
        return { rows: this.listActiveAgentConflicts(values) as T[] };
      case "group-provisioning-job-acquire-lease":
        return { rows: this.acquireLease(values) as T[] };
      case "group-provisioning-job-release-lease":
        return { rows: this.releaseLease(values) as T[] };
      case "group-provisioning-job-release-stale-leases":
        return { rows: [], rowCount: this.releaseStaleLeases(values) };
      case "group-provisioning-job-update-after-lease":
        return { rows: this.updateJob(values) as T[] };
      case "group-provisioning-job-cancel":
        return { rows: this.cancelJob(values) as T[] };
      case "group-provisioning-job-prepare-retry":
        return { rows: this.prepareRetry(values) as T[] };
      case "agent-template-instance-insert":
        this.agentTemplateInstances.push({
          agent_principal_id: values[0],
          role_template_id: values[1],
          role_template_version: values[2],
          instruction_status: values[3],
          setup_summary: JSON.parse(values[4] as string),
          workspace_mode: values[5],
          selected_skills: JSON.parse(values[6] as string),
          originating_job_id: values[7],
        });
        return { rows: [] };
      case "group-template-instance-insert":
        this.groupTemplateInstances.push({
          group_conversation_id: values[0],
          group_template_id: values[1],
          group_template_version: values[2],
          mission: values[3],
          workflow: JSON.parse(values[4] as string),
          kickoff_text: values[5],
          output_contract: JSON.parse(values[6] as string),
          originating_job_id: values[7],
        });
        return { rows: [] };
      case "group-template-role-assignment-insert":
        this.roleAssignments.push({
          id: values[0],
          group_conversation_id: values[1],
          slot_id: values[2],
          required: values[3],
          action: values[4],
          agent_principal_id: values[5],
          role_template_id: values[6],
          role_template_version: values[7],
          instruction_status: values[8],
          originating_job_id: values[9],
        });
        return { rows: [] };
      default:
        throw new Error(`unhandled query in fake client: ${String(name)}`);
    }
  }

  private insertJob(values: readonly unknown[]): JobRow {
    const now = "2026-05-19T00:00:00.000Z";
    const row: JobRow = {
      id: values[0] as string,
      company_id: values[1] as string,
      requested_by: values[2] as string,
      group_template_id: values[3] as string,
      group_template_version: values[4] as "1.0.0",
      status: values[5] as string,
      idempotency_key: values[6] as string,
      plan_json: JSON.parse(values[7] as string),
      step_results_json: {},
      involved_agent_ids: (values[8] as string[] | undefined) ?? [],
      created_agent_ids: [],
      created_group_id: null,
      error_summary: null,
      lease_owner: null,
      lease_token: null,
      lease_expires_at: null,
      created_at: now,
      updated_at: now,
      completed_at: null,
      canceled_at: null,
    };
    this.jobs.push(row);
    return row;
  }

  private acquireLease(values: readonly unknown[]): JobRow[] {
    const [jobId, leaseOwner, leaseToken, nowRaw, leaseMsRaw, terminalStatusesRaw] = values as [
      string,
      string,
      string,
      string,
      number,
      string[],
    ];
    const job = this.jobs.find((candidate) => candidate.id === jobId);
    if (!job || terminalStatusesRaw.includes(job.status)) return [];
    if (job.lease_token && job.lease_expires_at && Date.parse(job.lease_expires_at) > Date.parse(nowRaw)) return [];
    job.lease_owner = leaseOwner;
    job.lease_token = leaseToken;
    job.lease_expires_at = new Date(Date.parse(nowRaw) + leaseMsRaw).toISOString();
    job.updated_at = nowRaw;
    return [job];
  }

  private releaseLease(values: readonly unknown[]): JobRow[] {
    const [jobId, leaseToken, nowRaw] = values as [string, string, string];
    const job = this.jobs.find((candidate) => candidate.id === jobId && candidate.lease_token === leaseToken);
    if (!job) return [];
    job.lease_owner = null;
    job.lease_token = null;
    job.lease_expires_at = null;
    job.updated_at = nowRaw;
    return [job];
  }

  private releaseStaleLeases(values: readonly unknown[]): number {
    const [nowRaw, jobId] = values as [string, string | null];
    let released = 0;
    for (const job of this.jobs) {
      if (jobId && job.id !== jobId) continue;
      if (job.lease_token && job.lease_expires_at && Date.parse(job.lease_expires_at) <= Date.parse(nowRaw)) {
        job.lease_owner = null;
        job.lease_token = null;
        job.lease_expires_at = null;
        job.updated_at = nowRaw;
        released += 1;
      }
    }
    return released;
  }

  private listActiveAgentConflicts(values: readonly unknown[]): JobRow[] {
    const [companyId, agentIdsRaw, activeStatusesRaw] = values as [string, string[], string[]];
    const agentIds = new Set(agentIdsRaw);
    return this.jobs.filter((job) =>
      job.company_id === companyId
      && activeStatusesRaw.includes(job.status)
      && (
        job.involved_agent_ids.some((agentId) => agentIds.has(agentId))
        || job.created_agent_ids.some((agentId) => agentIds.has(agentId))
      ),
    );
  }

  private updateJob(values: readonly unknown[]): JobRow[] {
    const [
      jobId,
      leaseToken,
      status,
      stepResultsJson,
      involvedAgentIds,
      createdAgentIds,
      createdGroupId,
      hasErrorSummary,
      errorSummary,
      nowRaw,
      expectedStatus,
      terminalStatusesRaw,
    ] = values as [
      string,
      string,
      string | null,
      string | null,
      string[] | null,
      string[] | null,
      string | null,
      boolean,
      string | null,
      string,
      string,
      string[],
    ];
    const job = this.jobs.find((candidate) => candidate.id === jobId);
    if (!job || job.lease_token !== leaseToken) return [];
    if (!job.lease_expires_at || Date.parse(job.lease_expires_at) <= Date.parse(nowRaw)) return [];
    if (job.status !== expectedStatus || terminalStatusesRaw.includes(job.status)) return [];
    if (status) job.status = status;
    if (stepResultsJson) job.step_results_json = JSON.parse(stepResultsJson);
    if (involvedAgentIds) job.involved_agent_ids = involvedAgentIds;
    if (createdAgentIds) job.created_agent_ids = createdAgentIds;
    if (createdGroupId) job.created_group_id = createdGroupId;
    if (hasErrorSummary) job.error_summary = errorSummary;
    if (status === "completed" || status === "completed_with_warning") job.completed_at = nowRaw;
    if (status === "canceled") job.canceled_at = nowRaw;
    job.updated_at = nowRaw;
    return [job];
  }

  private cancelJob(values: readonly unknown[]): JobRow[] {
    const [
      jobId,
      leaseToken,
      hasErrorSummary,
      errorSummary,
      nowRaw,
      expectedStatus,
      terminalStatusesRaw,
    ] = values as [string, string | null, boolean, string | null, string, string, string[]];
    const job = this.jobs.find((candidate) => candidate.id === jobId);
    if (!job || job.status !== expectedStatus || terminalStatusesRaw.includes(job.status)) return [];
    if (!this.canUserActionMutateLease(job, leaseToken, nowRaw)) return [];
    job.status = "canceled";
    if (hasErrorSummary) job.error_summary = errorSummary;
    job.lease_owner = null;
    job.lease_token = null;
    job.lease_expires_at = null;
    job.canceled_at = nowRaw;
    job.updated_at = nowRaw;
    return [job];
  }

  private prepareRetry(values: readonly unknown[]): JobRow[] {
    const [
      jobId,
      leaseToken,
      nextStatus,
      nowRaw,
      expectedStatus,
      terminalStatusesRaw,
    ] = values as [string, string | null, string, string, string, string[]];
    const job = this.jobs.find((candidate) => candidate.id === jobId);
    if (!job || job.status !== expectedStatus || terminalStatusesRaw.includes(job.status)) return [];
    if (!this.canUserActionMutateLease(job, leaseToken, nowRaw)) return [];
    job.status = nextStatus;
    job.error_summary = null;
    job.updated_at = nowRaw;
    return [job];
  }

  private canUserActionMutateLease(job: JobRow, leaseToken: string | null, nowRaw: string): boolean {
    if (leaseToken) return job.lease_token === leaseToken;
    return !job.lease_token || !job.lease_expires_at || Date.parse(job.lease_expires_at) <= Date.parse(nowRaw);
  }
}
