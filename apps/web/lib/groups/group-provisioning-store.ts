import type {
  GroupTemplateWorkflow,
  InstructionStatus,
  OutputContract,
  TemplateVersion,
} from "./team-templates";

export const GROUP_PROVISIONING_JOB_STATUSES = [
  "validating",
  "creating_agents",
  "creating_group",
  "adding_members",
  "posting_kickoff",
  "completed",
  "completed_with_warning",
  "partial_failure",
  "failed_validation",
  "failed",
  "rolled_back",
  "canceled",
] as const;

export type GroupProvisioningJobStatus = (typeof GROUP_PROVISIONING_JOB_STATUSES)[number];

export const TERMINAL_GROUP_PROVISIONING_JOB_STATUSES = [
  "completed",
  "rolled_back",
  "canceled",
] as const satisfies readonly GroupProvisioningJobStatus[];

export const ACTIVE_GROUP_PROVISIONING_JOB_STATUSES = [
  "validating",
  "creating_agents",
  "creating_group",
  "adding_members",
  "posting_kickoff",
  "partial_failure",
  "failed",
] as const satisfies readonly GroupProvisioningJobStatus[];

export const GROUP_PROVISIONING_STATUS_TRANSITIONS: Record<GroupProvisioningJobStatus, readonly GroupProvisioningJobStatus[]> = {
  validating: ["creating_agents", "failed_validation", "canceled"],
  creating_agents: ["creating_group", "partial_failure", "failed", "rolled_back", "canceled"],
  creating_group: ["adding_members", "partial_failure", "failed", "rolled_back", "canceled"],
  adding_members: ["posting_kickoff", "partial_failure", "failed", "rolled_back", "canceled"],
  posting_kickoff: ["completed", "completed_with_warning", "failed", "canceled"],
  completed: [],
  completed_with_warning: ["posting_kickoff"],
  partial_failure: ["validating", "creating_agents", "creating_group", "adding_members", "posting_kickoff", "failed", "rolled_back", "canceled"],
  failed_validation: ["validating", "canceled"],
  failed: ["validating", "creating_agents", "creating_group", "adding_members", "posting_kickoff", "rolled_back", "canceled"],
  rolled_back: [],
  canceled: [],
};

export type GroupProvisioningRoleAction = "created" | "reused" | "skipped";
export type AgentTemplateWorkspaceMode = "generated" | "custom" | "existing" | "none";

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

export type QueryStatement = string | { name?: string; text: string };

export type QueryClient = {
  query<T extends object = Record<string, unknown>>(
    statement: QueryStatement,
    values?: readonly unknown[],
  ): Promise<{ rows: T[]; rowCount?: number | null }>;
};

export type GroupProvisioningJob = {
  id: string;
  companyId: string;
  requestedBy: string;
  groupTemplateId: string;
  groupTemplateVersion: TemplateVersion;
  status: GroupProvisioningJobStatus;
  idempotencyKey: string;
  planJson: JsonValue;
  stepResultsJson: JsonValue;
  involvedAgentIds: string[];
  createdAgentIds: string[];
  createdGroupId: string | null;
  errorSummary: string | null;
  leaseOwner: string | null;
  leaseToken: string | null;
  leaseExpiresAt: Date | string | null;
  createdAt: Date | string;
  updatedAt: Date | string;
  completedAt: Date | string | null;
  canceledAt: Date | string | null;
};

export type CreateGroupProvisioningJobInput = {
  id: string;
  companyId: string;
  requestedBy: string;
  groupTemplateId: string;
  groupTemplateVersion: TemplateVersion;
  idempotencyKey: string;
  planJson: JsonValue;
  involvedAgentIds?: string[];
  initialStatus?: GroupProvisioningJobStatus;
};

export type UpdateGroupProvisioningJobInput = {
  jobId: string;
  leaseToken: string;
  status?: GroupProvisioningJobStatus;
  planJson?: JsonValue;
  stepResultsJson?: JsonValue;
  involvedAgentIds?: string[];
  createdAgentIds?: string[];
  createdGroupId?: string | null;
  errorSummary?: string | null;
  now?: Date;
};

export type AgentTemplateInstanceInput = {
  agentPrincipalId: string;
  roleTemplateId: string;
  roleTemplateVersion: TemplateVersion;
  instructionStatus: InstructionStatus;
  setupSummary?: JsonValue;
  workspaceMode: AgentTemplateWorkspaceMode;
  selectedSkills?: string[];
  originatingJobId?: string | null;
};

export type GroupTemplateInstanceInput = {
  groupConversationId: string;
  groupTemplateId: string;
  groupTemplateVersion: TemplateVersion;
  mission: string;
  workflow: GroupTemplateWorkflow;
  kickoffText: string;
  outputContract: OutputContract;
  originatingJobId?: string | null;
};

type GroupTemplateRoleAssignmentBaseInput = {
  id: string;
  groupConversationId: string;
  slotId: string;
  required: boolean;
  roleTemplateId: string;
  roleTemplateVersion: TemplateVersion;
  originatingJobId?: string | null;
};

export type GroupTemplateRoleAssignmentInput =
  | (GroupTemplateRoleAssignmentBaseInput & {
      action: "created" | "reused";
      agentPrincipalId: string;
      instructionStatus: InstructionStatus;
    })
  | (GroupTemplateRoleAssignmentBaseInput & {
      action: "skipped";
      agentPrincipalId?: null;
      instructionStatus?: null;
    });

const STATUS_SET = new Set<GroupProvisioningJobStatus>(GROUP_PROVISIONING_JOB_STATUSES);
const TERMINAL_STATUS_SET = new Set<GroupProvisioningJobStatus>(TERMINAL_GROUP_PROVISIONING_JOB_STATUSES);
const SENSITIVE_KEY_PATTERN = /(^|[_-])(token|secret|password|credential|private[_-]?key|signing[_-]?secret|bearer|api[_-]?key|session)([_-]|$)/i;

export function createGroupProvisioningStore(client: QueryClient) {
  return {
    async createJob(input: CreateGroupProvisioningJobInput): Promise<GroupProvisioningJob> {
      assertGroupProvisioningJobStatus(input.initialStatus ?? "validating");
      const planJson = sanitizeProvisioningJson(input.planJson, "planJson");
      const result = await client.query<GroupProvisioningJobRow>(
        {
          name: "group-provisioning-job-create",
          text: `
            INSERT INTO group_provisioning_job (
              id,
              company_id,
              requested_by,
              group_template_id,
              group_template_version,
              status,
              idempotency_key,
              plan_json,
              involved_agent_ids
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb, $9::text[])
            RETURNING *
          `,
        },
        [
          input.id,
          input.companyId,
          input.requestedBy,
          input.groupTemplateId,
          input.groupTemplateVersion,
          input.initialStatus ?? "validating",
          input.idempotencyKey,
          JSON.stringify(planJson),
          input.involvedAgentIds ?? [],
        ],
      );
      return requireJob(result.rows[0], "createJob");
    },

    async createJobByIdempotencyKey(input: CreateGroupProvisioningJobInput): Promise<GroupProvisioningJob> {
      assertGroupProvisioningJobStatus(input.initialStatus ?? "validating");
      const planJson = sanitizeProvisioningJson(input.planJson, "planJson");
      const result = await client.query<GroupProvisioningJobRow>(
        {
          name: "group-provisioning-job-create-idempotent",
          text: `
            INSERT INTO group_provisioning_job (
              id,
              company_id,
              requested_by,
              group_template_id,
              group_template_version,
              status,
              idempotency_key,
              plan_json,
              involved_agent_ids
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb, $9::text[])
            ON CONFLICT (company_id, idempotency_key) DO NOTHING
            RETURNING *
          `,
        },
        [
          input.id,
          input.companyId,
          input.requestedBy,
          input.groupTemplateId,
          input.groupTemplateVersion,
          input.initialStatus ?? "validating",
          input.idempotencyKey,
          JSON.stringify(planJson),
          input.involvedAgentIds ?? [],
        ],
      );
      if (result.rows[0]) return mapJobRow(result.rows[0]);
      const existing = await this.getJobByIdempotencyKey(input.companyId, input.idempotencyKey);
      if (!existing) throw new Error("idempotent job insert conflicted but no existing job was found");
      return existing;
    },

    async getJob(jobId: string): Promise<GroupProvisioningJob | null> {
      const result = await client.query<GroupProvisioningJobRow>(
        {
          name: "group-provisioning-job-get",
          text: "SELECT * FROM group_provisioning_job WHERE id = $1",
        },
        [jobId],
      );
      return result.rows[0] ? mapJobRow(result.rows[0]) : null;
    },

    async getJobByIdempotencyKey(companyId: string, idempotencyKey: string): Promise<GroupProvisioningJob | null> {
      const result = await client.query<GroupProvisioningJobRow>(
        {
          name: "group-provisioning-job-get-by-idempotency-key",
          text: "SELECT * FROM group_provisioning_job WHERE company_id = $1 AND idempotency_key = $2",
        },
        [companyId, idempotencyKey],
      );
      return result.rows[0] ? mapJobRow(result.rows[0]) : null;
    },

    async listActiveJobsForAgentIds(input: { companyId: string; agentIds: string[] }): Promise<GroupProvisioningJob[]> {
      if (input.agentIds.length === 0) return [];
      const result = await client.query<GroupProvisioningJobRow>(
        {
          name: "group-provisioning-job-list-active-agent-conflicts",
          text: `
            SELECT *
              FROM group_provisioning_job
             WHERE company_id = $1
               AND status = ANY($3::text[])
               AND (
                 involved_agent_ids && $2::text[]
                 OR created_agent_ids && $2::text[]
               )
             ORDER BY created_at DESC
          `,
        },
        [input.companyId, input.agentIds, [...ACTIVE_GROUP_PROVISIONING_JOB_STATUSES]],
      );
      return result.rows.map(mapJobRow);
    },

    async acquireLease(input: {
      jobId: string;
      leaseOwner: string;
      leaseToken: string;
      leaseMs: number;
      now?: Date;
    }): Promise<GroupProvisioningJob | null> {
      const now = input.now ?? new Date();
      const result = await client.query<GroupProvisioningJobRow>(
        {
          name: "group-provisioning-job-acquire-lease",
          text: `
            UPDATE group_provisioning_job
               SET lease_owner = $2,
                   lease_token = $3,
                   lease_expires_at = $4::timestamptz + ($5::text || ' milliseconds')::interval,
                   updated_at = $4::timestamptz
             WHERE id = $1
               AND status <> ALL($6::text[])
               AND (lease_token IS NULL OR lease_expires_at IS NULL OR lease_expires_at <= $4::timestamptz)
            RETURNING *
          `,
        },
        [input.jobId, input.leaseOwner, input.leaseToken, now.toISOString(), input.leaseMs, [...TERMINAL_STATUS_SET]],
      );
      return result.rows[0] ? mapJobRow(result.rows[0]) : null;
    },

    async releaseLease(input: { jobId: string; leaseToken: string; now?: Date }): Promise<GroupProvisioningJob | null> {
      const result = await client.query<GroupProvisioningJobRow>(
        {
          name: "group-provisioning-job-release-lease",
          text: `
            UPDATE group_provisioning_job
               SET lease_owner = NULL,
                   lease_token = NULL,
                   lease_expires_at = NULL,
                   updated_at = $3::timestamptz
             WHERE id = $1
               AND lease_token = $2
            RETURNING *
          `,
        },
        [input.jobId, input.leaseToken, (input.now ?? new Date()).toISOString()],
      );
      return result.rows[0] ? mapJobRow(result.rows[0]) : null;
    },

    async releaseStaleLeases(input: { now?: Date; jobId?: string } = {}): Promise<number> {
      const result = await client.query(
        {
          name: "group-provisioning-job-release-stale-leases",
          text: `
            UPDATE group_provisioning_job
               SET lease_owner = NULL,
                   lease_token = NULL,
                   lease_expires_at = NULL,
                   updated_at = $1::timestamptz
             WHERE lease_token IS NOT NULL
               AND lease_expires_at <= $1::timestamptz
               AND ($2::text IS NULL OR id = $2::text)
          `,
        },
        [(input.now ?? new Date()).toISOString(), input.jobId ?? null],
      );
      return result.rowCount ?? 0;
    },

    async updateJobAfterLease(input: UpdateGroupProvisioningJobInput): Promise<GroupProvisioningJob> {
      if (input.status) assertGroupProvisioningJobStatus(input.status);
      if (input.planJson !== undefined) sanitizeProvisioningJson(input.planJson, "planJson");
      if (input.stepResultsJson !== undefined) sanitizeProvisioningJson(input.stepResultsJson, "stepResultsJson");
      const current = await this.getJob(input.jobId);
      if (!current) throw new Error(`group provisioning job not found: ${input.jobId}`);
      if (input.status) assertGroupProvisioningStatusTransition(current.status, input.status);
      const now = input.now ?? new Date();

      const result = await client.query<GroupProvisioningJobRow>(
        {
          name: "group-provisioning-job-update-after-lease",
          text: `
            UPDATE group_provisioning_job
               SET status = COALESCE($3::text, status),
                   plan_json = COALESCE($13::jsonb, plan_json),
                   step_results_json = COALESCE($4::jsonb, step_results_json),
                   involved_agent_ids = COALESCE($5::text[], involved_agent_ids),
                   created_agent_ids = COALESCE($6::text[], created_agent_ids),
                   created_group_id = COALESCE($7::text, created_group_id),
                   error_summary = CASE WHEN $8::boolean THEN $9::text ELSE error_summary END,
                   completed_at = CASE
                     WHEN $3::text IN ('completed', 'completed_with_warning') THEN $10::timestamptz
                     ELSE completed_at
                   END,
                   canceled_at = CASE
                     WHEN $3::text = 'canceled' THEN $10::timestamptz
                     ELSE canceled_at
                   END,
                   updated_at = $10::timestamptz
             WHERE id = $1
               AND lease_token = $2::text
               AND lease_expires_at > $10::timestamptz
               AND status = $11::text
               AND status <> ALL($12::text[])
            RETURNING *
          `,
        },
        [
          input.jobId,
          input.leaseToken ?? null,
          input.status ?? null,
          input.stepResultsJson === undefined ? null : JSON.stringify(input.stepResultsJson),
          input.involvedAgentIds ?? null,
          input.createdAgentIds ?? null,
          input.createdGroupId ?? null,
          input.errorSummary !== undefined,
          input.errorSummary ?? null,
          now.toISOString(),
          current.status,
          [...TERMINAL_STATUS_SET],
          input.planJson === undefined ? null : JSON.stringify(input.planJson),
        ],
      );
      return requireJob(result.rows[0], "updateJobAfterLease");
    },

    async cancelJob(input: { jobId: string; leaseToken?: string; errorSummary?: string | null; now?: Date }): Promise<GroupProvisioningJob> {
      const current = await this.getJob(input.jobId);
      if (!current) throw new Error(`group provisioning job not found: ${input.jobId}`);
      assertGroupProvisioningStatusTransition(current.status, "canceled");
      const now = input.now ?? new Date();
      const result = await client.query<GroupProvisioningJobRow>(
        {
          name: "group-provisioning-job-cancel",
          text: `
            UPDATE group_provisioning_job
               SET status = 'canceled',
                   error_summary = CASE WHEN $3::boolean THEN $4::text ELSE error_summary END,
                   lease_owner = NULL,
                   lease_token = NULL,
                   lease_expires_at = NULL,
                   canceled_at = $5::timestamptz,
                   updated_at = $5::timestamptz
             WHERE id = $1
               AND status = $6::text
               AND status <> ALL($7::text[])
               AND (
                 ($2::text IS NOT NULL AND lease_token = $2::text)
                 OR ($2::text IS NULL AND (lease_token IS NULL OR lease_expires_at IS NULL OR lease_expires_at <= $5::timestamptz))
               )
            RETURNING *
          `,
        },
        [
          input.jobId,
          input.leaseToken ?? null,
          input.errorSummary !== undefined,
          input.errorSummary ?? null,
          now.toISOString(),
          current.status,
          [...TERMINAL_STATUS_SET],
        ],
      );
      return requireJob(result.rows[0], "cancelJob");
    },

    async prepareRetry(input: { jobId: string; leaseToken?: string; nextStatus: GroupProvisioningJobStatus; now?: Date }): Promise<GroupProvisioningJob> {
      assertGroupProvisioningJobStatus(input.nextStatus);
      const current = await this.getJob(input.jobId);
      if (!current) throw new Error(`group provisioning job not found: ${input.jobId}`);
      assertGroupProvisioningStatusTransition(current.status, input.nextStatus);
      const now = input.now ?? new Date();
      const result = await client.query<GroupProvisioningJobRow>(
        {
          name: "group-provisioning-job-prepare-retry",
          text: `
            UPDATE group_provisioning_job
               SET status = $3::text,
                   error_summary = NULL,
                   updated_at = $4::timestamptz
             WHERE id = $1
               AND status = $5::text
               AND status <> ALL($6::text[])
               AND (
                 ($2::text IS NOT NULL AND lease_token = $2::text)
                 OR ($2::text IS NULL AND (lease_token IS NULL OR lease_expires_at IS NULL OR lease_expires_at <= $4::timestamptz))
               )
            RETURNING *
          `,
        },
        [
          input.jobId,
          input.leaseToken ?? null,
          input.nextStatus,
          now.toISOString(),
          current.status,
          [...TERMINAL_STATUS_SET],
        ],
      );
      return requireJob(result.rows[0], "prepareRetry");
    },

    async insertAgentTemplateInstance(input: AgentTemplateInstanceInput): Promise<void> {
      sanitizeProvisioningJson(input.setupSummary ?? {}, "setupSummary");
      await client.query(
        {
          name: "agent-template-instance-insert",
          text: `
            INSERT INTO agent_template_instance (
              agent_principal_id,
              role_template_id,
              role_template_version,
              instruction_status,
              setup_summary,
              workspace_mode,
              selected_skills,
              originating_job_id
            )
            VALUES ($1, $2, $3, $4, $5::jsonb, $6, $7::jsonb, $8)
          `,
        },
        [
          input.agentPrincipalId,
          input.roleTemplateId,
          input.roleTemplateVersion,
          input.instructionStatus,
          JSON.stringify(input.setupSummary ?? {}),
          input.workspaceMode,
          JSON.stringify(input.selectedSkills ?? []),
          input.originatingJobId ?? null,
        ],
      );
    },

    async insertGroupTemplateInstance(input: GroupTemplateInstanceInput): Promise<void> {
      sanitizeProvisioningJson(input.workflow as unknown as JsonValue, "workflow");
      sanitizeProvisioningJson(input.outputContract as unknown as JsonValue, "outputContract");
      await client.query(
        {
          name: "group-template-instance-insert",
          text: `
            INSERT INTO group_template_instance (
              group_conversation_id,
              group_template_id,
              group_template_version,
              mission,
              workflow,
              kickoff_text,
              output_contract,
              originating_job_id
            )
            VALUES ($1, $2, $3, $4, $5::jsonb, $6, $7::jsonb, $8)
            ON CONFLICT (group_conversation_id) DO UPDATE SET
              group_template_id = EXCLUDED.group_template_id,
              group_template_version = EXCLUDED.group_template_version,
              mission = EXCLUDED.mission,
              workflow = EXCLUDED.workflow,
              kickoff_text = EXCLUDED.kickoff_text,
              output_contract = EXCLUDED.output_contract,
              originating_job_id = EXCLUDED.originating_job_id
          `,
        },
        [
          input.groupConversationId,
          input.groupTemplateId,
          input.groupTemplateVersion,
          input.mission,
          JSON.stringify(input.workflow),
          input.kickoffText,
          JSON.stringify(input.outputContract),
          input.originatingJobId ?? null,
        ],
      );
    },

    async insertRoleAssignment(input: GroupTemplateRoleAssignmentInput): Promise<void> {
      const action = input.action as GroupProvisioningRoleAction;
      const agentPrincipalId = "agentPrincipalId" in input ? input.agentPrincipalId : null;
      const instructionStatus = "instructionStatus" in input ? input.instructionStatus : null;

      if (action === "skipped" && agentPrincipalId) {
        throw new Error("skipped role assignments cannot reference an agent");
      }
      if (action === "skipped" && instructionStatus) {
        throw new Error("skipped role assignments cannot record instruction status");
      }
      if (action !== "skipped" && !agentPrincipalId) {
        throw new Error(`${action} role assignments must reference an agent`);
      }
      if (action !== "skipped" && !instructionStatus) {
        throw new Error(`${action} role assignments must record instruction status`);
      }
      await client.query(
        {
          name: "group-template-role-assignment-insert",
          text: `
            INSERT INTO group_template_role_assignment (
              id,
              group_conversation_id,
              slot_id,
              required,
              action,
              agent_principal_id,
              role_template_id,
              role_template_version,
              instruction_status,
              originating_job_id
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (id) DO UPDATE SET
              group_conversation_id = EXCLUDED.group_conversation_id,
              slot_id = EXCLUDED.slot_id,
              required = EXCLUDED.required,
              action = EXCLUDED.action,
              agent_principal_id = EXCLUDED.agent_principal_id,
              role_template_id = EXCLUDED.role_template_id,
              role_template_version = EXCLUDED.role_template_version,
              instruction_status = EXCLUDED.instruction_status,
              originating_job_id = EXCLUDED.originating_job_id
          `,
        },
        [
          input.id,
          input.groupConversationId,
          input.slotId,
          input.required,
          action,
          agentPrincipalId ?? null,
          input.roleTemplateId,
          input.roleTemplateVersion,
          instructionStatus ?? null,
          input.originatingJobId ?? null,
        ],
      );
    },
  };
}

export function assertGroupProvisioningJobStatus(status: string): asserts status is GroupProvisioningJobStatus {
  if (!STATUS_SET.has(status as GroupProvisioningJobStatus)) {
    throw new Error(`unknown group provisioning job status: ${status}`);
  }
}

export function assertGroupProvisioningStatusTransition(
  currentStatus: GroupProvisioningJobStatus,
  nextStatus: GroupProvisioningJobStatus,
): void {
  if (currentStatus === nextStatus) return;
  if (!GROUP_PROVISIONING_STATUS_TRANSITIONS[currentStatus].includes(nextStatus)) {
    throw new Error(`invalid group provisioning status transition: ${currentStatus} -> ${nextStatus}`);
  }
}

export function sanitizeProvisioningJson<T extends JsonValue>(value: T, label = "json"): T {
  visitJsonForSensitiveKeys(value, label);
  return value;
}

function visitJsonForSensitiveKeys(value: JsonValue, path: string): void {
  if (Array.isArray(value)) {
    value.forEach((item, index) => visitJsonForSensitiveKeys(item, `${path}[${index}]`));
    return;
  }
  if (!value || typeof value !== "object") return;

  for (const [key, child] of Object.entries(value)) {
    const childPath = `${path}.${key}`;
    const normalizedKey = key.replace(/([a-z0-9])([A-Z])/g, "$1_$2");
    if (SENSITIVE_KEY_PATTERN.test(normalizedKey)) {
      throw new Error(`refusing to store sensitive provisioning key at ${childPath}`);
    }
    visitJsonForSensitiveKeys(child, childPath);
  }
}

type GroupProvisioningJobRow = {
  id: string;
  company_id: string;
  requested_by: string;
  group_template_id: string;
  group_template_version: TemplateVersion;
  status: GroupProvisioningJobStatus;
  idempotency_key: string;
  plan_json: JsonValue;
  step_results_json: JsonValue;
  involved_agent_ids: string[];
  created_agent_ids: string[];
  created_group_id: string | null;
  error_summary: string | null;
  lease_owner: string | null;
  lease_token: string | null;
  lease_expires_at: Date | string | null;
  created_at: Date | string;
  updated_at: Date | string;
  completed_at: Date | string | null;
  canceled_at: Date | string | null;
};

function mapJobRow(row: GroupProvisioningJobRow): GroupProvisioningJob {
  assertGroupProvisioningJobStatus(row.status);
  return {
    id: row.id,
    companyId: row.company_id,
    requestedBy: row.requested_by,
    groupTemplateId: row.group_template_id,
    groupTemplateVersion: row.group_template_version,
    status: row.status,
    idempotencyKey: row.idempotency_key,
    planJson: row.plan_json,
    stepResultsJson: row.step_results_json,
    involvedAgentIds: row.involved_agent_ids,
    createdAgentIds: row.created_agent_ids,
    createdGroupId: row.created_group_id,
    errorSummary: row.error_summary,
    leaseOwner: row.lease_owner,
    leaseToken: row.lease_token,
    leaseExpiresAt: row.lease_expires_at,
    createdAt: row.created_at,
    updatedAt: row.updated_at,
    completedAt: row.completed_at,
    canceledAt: row.canceled_at,
  };
}

function requireJob(row: GroupProvisioningJobRow | undefined, operation: string): GroupProvisioningJob {
  if (!row) throw new Error(`${operation} did not return a group provisioning job`);
  return mapJobRow(row);
}
