import crypto from "node:crypto";

import {
  AgentProvisioningError,
  defaultRoleTemplateProvenanceWriter,
  provisionAgent,
  type AgentProvisioningStepRecord,
  type AgentProvisioningStepRecorder,
  type ProvisionRequestBody,
} from "../agents/agent-provisioning";
import {
  addGroupMembers,
  createGroup,
  fetchConsoleSnapshot,
  fetchRuntimeBindings,
  sendMessage,
  upsertRuntimePolicy,
  type ChatMessage,
} from "../api/choruz-api";
import {
  GROUP_PROVISIONING_STATUS_CONTRACT,
  type ContractRoleSlotPlan,
  type GroupLaunchPlanContract,
  type GroupLaunchPlanWorkflow,
  type GroupProvisioningIssue,
  type GroupProvisioningJobContract,
  type GroupProvisioningStepResult,
  type ProgressStep,
  type ProvisioningJobCancelRequest,
  type ProvisioningJobCreationRequest,
  type ProvisioningJobRetryRequest,
  type RecoveryChoice,
} from "./group-provisioning-contract";
import { createGroupProvisioningStore } from "./group-provisioning-store";
import { postgresQueryClient } from "./group-provisioning-db";
import {
  getGroupTemplate,
  getRoleTemplate,
  type GroupTemplate,
  type InstructionStatus,
  type RoleTemplate,
  type TemplateVersion,
} from "./team-templates";
import { renderGroupKickoff, renderRoleInstructions, type GroupKickoffMember } from "./team-template-renderer";
import {
  validateGroupLaunchPlan,
  type ExistingAgentCandidate,
} from "./team-template-validation";
import type { DriverAvailabilityItem } from "../drivers/driver-availability";
import type {
  GroupProvisioningJob,
  GroupProvisioningJobStatus,
  JsonValue,
} from "./group-provisioning-store";

export type GroupProvisioningStore = ReturnType<typeof createGroupProvisioningStore>;

type MemberAgentResult = {
  action: "created" | "reused";
  roleSlotId: string;
  agentId: string;
  agentName: string;
  roleTemplateId: string;
  roleTemplateVersion: TemplateVersion;
  instructionStatus: InstructionStatus;
};

type AgentProvisioningStepMap = Record<string, AgentProvisioningStepRecord>;

type ProvisioningStepState = {
  results: GroupProvisioningStepResult[];
  issues: GroupProvisioningIssue[];
  agentSteps: AgentProvisioningStepMap;
};

export type GroupProvisioningRunnerDeps = {
  store: GroupProvisioningStore;
  now?: () => Date;
  newId?: () => string;
  leaseOwner?: string;
  leaseMs?: number;
  provisionAgent?: typeof provisionAgent;
  createGroup?: typeof createGroup;
  addGroupMembers?: typeof addGroupMembers;
  sendMessage?: typeof sendMessage;
  enableRoutingPolicy?: (input: {
    sessionToken: string;
    actorId: string;
    conversationId: string;
    idempotencyKey: string;
    coordinatorAgentId?: string;
  }) => Promise<void>;
  loadExistingAgentCandidates?: (input: {
    sessionToken: string;
    companyId: string;
    agentIds: string[];
  }) => Promise<ExistingAgentCandidate[]>;
  loadGeneratedAgentCleanupCandidates?: (input: {
    companyId: string;
    agentNames: string[];
  }) => Promise<ExistingAgentCandidate[]>;
  softDisableGeneratedAgents?: (input: {
    actorId: string;
    companyId: string;
    agentIds: string[];
  }) => Promise<void>;
  loadDriverAvailability?: () => Promise<DriverAvailabilityItem[]>;
};

export type CreateGroupProvisioningJobInput = {
  sessionToken: string;
  actorId: string;
  companyId: string;
  body: ProvisioningJobCreationRequest;
};

export type RunGroupProvisioningJobInput = {
  sessionToken: string;
  actorId: string;
  jobId: string;
  maxSteps?: number;
};

const DEFAULT_LEASE_MS = 30_000;
const MAX_BATCH_STEPS = 10;
const BLOCKING_REUSE_ISSUE_CODES = new Set([
  "existing_agent_disabled",
  "missing_runtime_binding",
  "runtime_binding_not_reusable",
]);
const RETRY_ROUTE_CHOICES = new Set<ProvisioningJobRetryRequest["choice"]>([
  "edit_plan",
  "retry_validation",
  "retry_agent_creation",
  "skip_optional_role",
  "retry_group_creation",
  "soft_delete_generated_agents",
  "retry_member_add",
  "replace_agent",
  "manual_invite",
  "retry_kickoff",
]);

export async function defaultGroupProvisioningStore(): Promise<GroupProvisioningStore> {
  return createGroupProvisioningStore(await postgresQueryClient());
}

export function createGroupProvisioningRunner(deps: GroupProvisioningRunnerDeps) {
  const now = deps.now ?? (() => new Date());
  const newId = deps.newId ?? (() => crypto.randomUUID());
  const leaseOwner = deps.leaseOwner ?? "web-group-provisioning-runner";
  const leaseMs = deps.leaseMs ?? DEFAULT_LEASE_MS;
  const provisionAgentFn = deps.provisionAgent ?? provisionAgent;
  const createGroupFn = deps.createGroup ?? createGroup;
  const addGroupMembersFn = deps.addGroupMembers ?? addGroupMembers;
  const sendMessageFn = deps.sendMessage ?? sendMessage;
  const enableRoutingPolicyFn = deps.enableRoutingPolicy ?? defaultEnableRoutingPolicy;
  const loadExistingAgentCandidatesFn = deps.loadExistingAgentCandidates ?? defaultLoadExistingAgentCandidates;
  const loadGeneratedAgentCleanupCandidatesFn = deps.loadGeneratedAgentCleanupCandidates;
  const softDisableGeneratedAgentsFn = deps.softDisableGeneratedAgents;
  const loadDriverAvailabilityFn = deps.loadDriverAvailability ?? (async () => []);

  async function createJob(input: CreateGroupProvisioningJobInput): Promise<GroupProvisioningJobContract> {
    const validation = await validatePlanForJob({
      plan: input.body.plan,
      companyId: input.companyId,
      sessionToken: input.sessionToken,
    });
    const { plan, issues } = validation;
    const jobId = newId();
    const created = await deps.store.createJobByIdempotencyKey({
      id: jobId,
      companyId: input.companyId,
      requestedBy: input.actorId,
      groupTemplateId: plan.groupTemplateId,
      groupTemplateVersion: plan.groupTemplateVersion,
      idempotencyKey: input.body.idempotencyKey,
      planJson: plan as unknown as JsonValue,
      involvedAgentIds: involvedAgentIdsFromPlan(plan),
      initialStatus: "validating",
    });
    if (created.id !== jobId) return toJobContract(created);
    if (issues.some((issue) => issue.severity === "error")) {
      const leaseToken = newId();
      const leased = await deps.store.acquireLease({
        jobId: created.id,
        leaseOwner,
        leaseToken,
        leaseMs,
        now: now(),
      });
      if (leased) {
        const failed = await deps.store.updateJobAfterLease({
          jobId: created.id,
          leaseToken,
          status: "failed_validation",
          stepResultsJson: stepState([], issues),
          errorSummary: "Launch plan validation failed.",
          now: now(),
        });
        await deps.store.releaseLease({ jobId: created.id, leaseToken, now: now() });
        return toJobContract(failed);
      }
    }
    return toJobContract(created);
  }

  async function getJob(jobId: string): Promise<GroupProvisioningJobContract | null> {
    const job = await deps.store.getJob(jobId);
    return job ? toJobContract(job) : null;
  }

  async function runJob(input: RunGroupProvisioningJobInput): Promise<GroupProvisioningJobContract | null> {
    await deps.store.releaseStaleLeases({ jobId: input.jobId, now: now() });
    const existing = await deps.store.getJob(input.jobId);
    if (!existing) return null;
    if (GROUP_PROVISIONING_STATUS_CONTRACT[existing.status].terminal || existing.status === "completed_with_warning") {
      return toJobContract(existing);
    }

    const leaseToken = newId();
    const leased = await deps.store.acquireLease({
      jobId: input.jobId,
      leaseOwner,
      leaseToken,
      leaseMs,
      now: now(),
    });
    if (!leased) return toJobContract(existing);

    let current = leased;
    const maxSteps = Math.max(1, Math.min(input.maxSteps ?? 1, MAX_BATCH_STEPS));
    try {
      for (let index = 0; index < maxSteps; index += 1) {
        const advanced = await advanceOneStep(current, input, leaseToken);
        current = advanced;
        if (GROUP_PROVISIONING_STATUS_CONTRACT[current.status].terminal) break;
        if (current.status === "partial_failure" || current.status === "failed" || current.status === "failed_validation") break;
      }
      return toJobContract(current);
    } finally {
      await deps.store.releaseLease({ jobId: input.jobId, leaseToken, now: now() });
    }
  }

  async function retryJob(jobId: string, request: ProvisioningJobRetryRequest): Promise<GroupProvisioningJobContract | null> {
    const job = await deps.store.getJob(jobId);
    if (!job) return null;
    if (!RETRY_ROUTE_CHOICES.has(request.choice)) return toJobContract(job);
    const allowedChoices = recoveryChoicesFor(job.status, readIssues(job));
    const allowedChoice = allowedChoices.find((choice) =>
      choice.id === request.choice
      && (!choice.roleSlotId || !request.roleSlotId || choice.roleSlotId === request.roleSlotId)
    );
    if (!allowedChoice) return toJobContract(job);
    const scopedRequest = {
      ...request,
      roleSlotId: request.roleSlotId ?? allowedChoice.roleSlotId,
    };
    if (request.choice === "soft_delete_generated_agents") {
      return softDeleteGeneratedAgentsForJob(job, "cleanup:recovery");
    }
    if (request.choice === "skip_optional_role" || request.choice === "manual_invite") {
      const roleSlotId = scopedRequest.roleSlotId ?? readIssues(job).at(-1)?.roleSlotId;
      const skipsMember = !!roleSlotId && memberAgentResults(readStepResults(job)).some((member) => member.roleSlotId === roleSlotId);
      const recovered = await markOptionalMemberOrRoleSkipped(job, scopedRequest);
      if (!recovered) return toJobContract(job);
      const retried = await deps.store.prepareRetry({
        jobId,
        nextStatus: skipsMember || scopedRequest.choice === "manual_invite" ? "adding_members" : "creating_agents",
        now: now(),
      });
      return toJobContract(retried);
    }
    const nextStatus = retryStatusFor(job.status, scopedRequest.choice);
    const retried = await deps.store.prepareRetry({ jobId, nextStatus, now: now() });
    return toJobContract(retried);
  }

  async function cancelJob(jobId: string, request: ProvisioningJobCancelRequest): Promise<GroupProvisioningJobContract | null> {
    const job = await deps.store.getJob(jobId);
    if (!job) return null;
    const leaseToken = newId();
    await deps.store.releaseStaleLeases({ jobId, now: now() });
    const leased = await deps.store.acquireLease({ jobId, leaseOwner, leaseToken, leaseMs, now: now() });
    if (!leased) return toJobContract(job);
    try {
      const priorResults = readStepResults(leased);
      const generatedAgentIds = await generatedAgentIdsForCleanup(leased, loadGeneratedAgentCleanupCandidatesFn);
      const hasSideEffects = generatedAgentIds.length > 0 || !!leased.createdGroupId || priorResults.some((result) => result.kind !== "cleanup");
      const cleanupResult = hasSideEffects
        ? await cleanupResultForJob(leased, "cleanup:cancel", request.choice === "soft_delete_generated_agents")
        : null;
      const residualResult = hasSideEffects ? residualAssetsForJob(leased) : null;
      const nextResults = [
        ...priorResults,
        ...(cleanupResult ? [cleanupResult] : []),
        ...(residualResult ? [residualResult] : []),
      ];
      const updated = await deps.store.updateJobAfterLease({
        jobId,
        leaseToken,
        stepResultsJson: stepState(nextResults, []),
        now: now(),
      });
      const canceled = await deps.store.cancelJob({
        jobId,
        leaseToken,
        errorSummary: request.reason ?? "User canceled group provisioning.",
        now: now(),
      });
      void updated;
      return toJobContract(canceled);
    } finally {
      await deps.store.releaseLease({ jobId, leaseToken, now: now() });
    }
  }

  async function softDeleteGeneratedAgentsForJob(
    job: GroupProvisioningJob,
    stepId: string,
  ): Promise<GroupProvisioningJobContract> {
    const leaseToken = newId();
    await deps.store.releaseStaleLeases({ jobId: job.id, now: now() });
    const leased = await deps.store.acquireLease({ jobId: job.id, leaseOwner, leaseToken, leaseMs, now: now() });
    if (!leased) return toJobContract(job);
    try {
      const priorResults = readStepResults(leased);
      const cleanupResult = await cleanupResultForJob(leased, stepId, true);
      const residualResult = residualAssetsForJob(leased);
      const nextStatus = cleanupResult.result === "failed"
        ? leased.status === "failed" ? undefined : "failed"
        : "rolled_back";
      const updated = await deps.store.updateJobAfterLease({
        jobId: job.id,
        leaseToken,
        status: nextStatus,
        stepResultsJson: stepState([...priorResults, cleanupResult, residualResult], cleanupResult.issue ? [cleanupResult.issue] : []),
        errorSummary: cleanupResult.issue?.message ?? null,
        now: now(),
      });
      return toJobContract(updated);
    } finally {
      await deps.store.releaseLease({ jobId: job.id, leaseToken, now: now() });
    }
  }

  async function cleanupResultForJob(
    job: GroupProvisioningJob,
    stepId: string,
    shouldSoftDelete: boolean,
  ): Promise<Extract<GroupProvisioningStepResult, { kind: "cleanup" }>> {
    const generatedAgentIds = await generatedAgentIdsForCleanup(job, loadGeneratedAgentCleanupCandidatesFn);
    const preservedAgentIds = reusedAgentIdsFromResults(readStepResults(job));
    if (!shouldSoftDelete || generatedAgentIds.length === 0) {
      return {
        kind: "cleanup",
        stepId,
        result: "none_needed",
        softDeletedAgentIds: [],
        preservedAgentIds,
      };
    }
    try {
      if (!softDisableGeneratedAgentsFn) {
        throw new Error("Generated-agent soft deletion is not configured.");
      }
      await softDisableGeneratedAgentsFn({
        actorId: job.requestedBy,
        companyId: job.companyId,
        agentIds: generatedAgentIds,
      });
      return {
        kind: "cleanup",
        stepId,
        result: "soft_deleted_generated_agents",
        softDeletedAgentIds: generatedAgentIds,
        preservedAgentIds,
      };
    } catch (error) {
      return {
        kind: "cleanup",
        stepId,
        result: "failed",
        softDeletedAgentIds: [],
        preservedAgentIds,
        issue: issueFromError("generated_agent_soft_delete_failed", error, { recoverable: true }),
      };
    }
  }

  async function markOptionalMemberOrRoleSkipped(
    job: GroupProvisioningJob,
    request: ProvisioningJobRetryRequest,
  ): Promise<GroupProvisioningJob | null> {
    const plan = assertPlan(job.planJson);
    const results = readStepResults(job);
    const issues = readIssues(job);
    const roleSlotId = request.roleSlotId ?? issues.at(-1)?.roleSlotId;
    if (!roleSlotId) return null;
    const rolePlan = plan.rolePlans.find((candidate) => candidate.slotId === roleSlotId);
    if (!rolePlan) return null;
    const member = memberAgentResults(results).find((candidate) => candidate.roleSlotId === roleSlotId);
    if (request.choice === "skip_optional_role" && rolePlanIsRequired(plan, roleSlotId)) return null;
    if (request.choice === "manual_invite" && !member) return null;
    const skipResult: GroupProvisioningStepResult = member
      ? {
          kind: "member_add",
          stepId: request.choice === "manual_invite"
            ? `member:add:${roleSlotId}:manual-invite`
            : `member:add:${roleSlotId}:skipped`,
          roleSlotId,
          agentId: member.agentId,
          result: "skipped",
        }
      : {
          kind: "skipped_optional_role",
          stepId: `role:skip:${roleSlotId}:recovery`,
          roleSlotId,
          roleTemplateId: rolePlan.roleTemplateId,
          reason: "recovery_choice",
        };
    const leaseToken = newId();
    await deps.store.releaseStaleLeases({ jobId: job.id, now: now() });
    const leased = await deps.store.acquireLease({ jobId: job.id, leaseOwner, leaseToken, leaseMs, now: now() });
    if (!leased) return null;
    const updated = await deps.store.updateJobAfterLease({
      jobId: job.id,
      leaseToken,
      stepResultsJson: stepState([...results, skipResult, residualAssetsForJob(job)], []),
      involvedAgentIds: involvedAgentIdsFromResults([...results, skipResult], plan),
      createdAgentIds: createdAgentIdsFromResults([...results, skipResult]),
      now: now(),
    });
    await deps.store.releaseLease({ jobId: job.id, leaseToken, now: now() });
    return updated;
  }

  async function advanceOneStep(
    job: GroupProvisioningJob,
    input: RunGroupProvisioningJobInput,
    leaseToken: string,
  ): Promise<GroupProvisioningJob> {
    const plan = assertPlan(job.planJson);
    const results = readStepResults(job);
    if (job.status === "validating") {
      const validation = await validatePlanForJob({
        plan,
        companyId: job.companyId,
        sessionToken: input.sessionToken,
        currentJobId: job.id,
      });
      const { plan: validatedPlan, issues } = validation;
      if (issues.some((issue) => issue.severity === "error")) {
        return deps.store.updateJobAfterLease({
          jobId: job.id,
          leaseToken,
          status: "failed_validation",
          planJson: validatedPlan as unknown as JsonValue,
          stepResultsJson: stepState(results, issues),
          errorSummary: "Launch plan validation failed.",
          now: now(),
        });
      }
      return deps.store.updateJobAfterLease({
        jobId: job.id,
        leaseToken,
        status: "creating_agents",
        planJson: validatedPlan as unknown as JsonValue,
        stepResultsJson: stepState(results, issues),
        errorSummary: null,
        now: now(),
      });
    }

    if (job.status === "creating_agents") {
      const nextRolePlan = plan.rolePlans.find((rolePlan) => !hasRoleResult(results, rolePlan.slotId));
      if (!nextRolePlan) {
        return deps.store.updateJobAfterLease({
          jobId: job.id,
          leaseToken,
          status: "creating_group",
          stepResultsJson: stepState(results, []),
          involvedAgentIds: involvedAgentIdsFromResults(results, plan),
          createdAgentIds: createdAgentIdsFromResults(results),
          now: now(),
        });
      }
      try {
        const result = await runRolePlan(job, plan, nextRolePlan, input, leaseToken);
        const latest = await deps.store.getJob(job.id);
        const latestState = latest ? readProvisioningState(latest) : readProvisioningState(job);
        const nextResults = [...latestState.results, result];
        return deps.store.updateJobAfterLease({
          jobId: job.id,
          leaseToken,
          stepResultsJson: stepState(nextResults, [], latestState.agentSteps),
          involvedAgentIds: involvedAgentIdsFromResults(nextResults, plan),
          createdAgentIds: createdAgentIdsFromResults(nextResults),
          now: now(),
        });
      } catch (error) {
        const issue = issueFromError(
          rolePlanIsRequired(plan, nextRolePlan.slotId) ? "required_agent_creation_failed" : "optional_agent_creation_failed",
          error,
          { recoverable: true, roleSlotId: nextRolePlan.slotId, roleTemplateId: nextRolePlan.roleTemplateId },
        );
        const latest = await deps.store.getJob(job.id);
        const latestState = latest ? readProvisioningState(latest) : readProvisioningState(job);
        return deps.store.updateJobAfterLease({
          jobId: job.id,
          leaseToken,
          status: "partial_failure",
          stepResultsJson: stepState(latestState.results, [issue], latestState.agentSteps),
          errorSummary: errorMessage(error),
          now: now(),
        });
      }
    }

    if (job.status === "creating_group") {
      const existingGroup = findResult(results, "created_group");
      if (existingGroup) {
        return deps.store.updateJobAfterLease({ jobId: job.id, leaseToken, status: "adding_members", now: now() });
      }
      try {
        const group = await createGroupFn(
          input.sessionToken,
          input.actorId,
          plan.groupName,
          [],
          job.companyId,
        );
        const result: GroupProvisioningStepResult = {
          kind: "created_group",
          stepId: "group:create",
          groupConversationId: group.id,
          groupName: plan.groupName,
        };
        return deps.store.updateJobAfterLease({
          jobId: job.id,
          leaseToken,
          status: "adding_members",
          stepResultsJson: stepState([...results, result], []),
          createdGroupId: group.id,
          now: now(),
        });
      } catch (error) {
        const issue = issueFromError("group_creation_failed", error, { recoverable: true });
        return deps.store.updateJobAfterLease({
          jobId: job.id,
          leaseToken,
          status: "failed",
          stepResultsJson: stepState([...results, residualAssetsForJob(job)], [issue]),
          errorSummary: errorMessage(error),
          now: now(),
        });
      }
    }

    if (job.status === "adding_members") {
      const groupId = job.createdGroupId ?? findResult(results, "created_group")?.groupConversationId;
      if (!groupId) {
        return deps.store.updateJobAfterLease({
          jobId: job.id,
          leaseToken,
          status: "failed",
          errorSummary: "Cannot add members before group creation succeeds.",
          now: now(),
        });
      }
      const memberCandidates = memberAgentResults(results);
      const nextMember = memberCandidates.find((candidate) => !hasMemberAddResult(results, candidate.roleSlotId));
      if (nextMember) {
        try {
          await addGroupMembersFn(
            input.sessionToken,
            input.actorId,
            groupId,
            [nextMember.agentId],
          );
          const nextResults: GroupProvisioningStepResult[] = [
            ...results,
            {
              kind: "member_add",
              stepId: `member:add:${nextMember.roleSlotId}`,
              roleSlotId: nextMember.roleSlotId,
              agentId: nextMember.agentId,
              result: "added",
            },
          ];
          await persistRoleAssignment(job, plan, groupId, nextMember);
          return deps.store.updateJobAfterLease({
            jobId: job.id,
            leaseToken,
            stepResultsJson: stepState(nextResults, []),
            now: now(),
          });
        } catch (error) {
          const issue = issueFromError(
            rolePlanIsRequired(plan, nextMember.roleSlotId) ? "required_member_add_failed" : "optional_member_add_failed",
            error,
            {
              recoverable: true,
              roleSlotId: nextMember.roleSlotId,
              agentId: nextMember.agentId,
            },
          );
          return deps.store.updateJobAfterLease({
            jobId: job.id,
            leaseToken,
            status: "partial_failure",
            stepResultsJson: stepState([
              ...results,
              {
                kind: "member_add",
                stepId: `member:add:${nextMember.roleSlotId}:failed`,
                roleSlotId: nextMember.roleSlotId,
                agentId: nextMember.agentId,
                result: "failed",
                issue,
              },
              residualAssetsForJob(job),
            ], [issue]),
            errorSummary: errorMessage(error),
            now: now(),
          });
        }
      }
      if (!findResult(results, "routing_policy_update")) {
        await persistGroupMetadata(job, plan, groupId, results);
        try {
          await enableRoutingPolicyFn({
            sessionToken: input.sessionToken,
            actorId: input.actorId,
            conversationId: groupId,
            idempotencyKey: idempotencyKey(job, "routing-policy", groupId),
            ...coordinatorRoutingPolicyInput(plan, results),
          });
          return deps.store.updateJobAfterLease({
            jobId: job.id,
            leaseToken,
            stepResultsJson: stepState([
              ...results,
              {
                kind: "routing_policy_update",
                stepId: "routing-policy:enable",
                groupConversationId: groupId,
                result: "enabled",
              },
            ], []),
            now: now(),
          });
        } catch (error) {
          return deps.store.updateJobAfterLease({
            jobId: job.id,
            leaseToken,
            status: "partial_failure",
            stepResultsJson: stepState(results, [issueFromError("routing_policy_failed", error, { recoverable: true })]),
            errorSummary: errorMessage(error),
            now: now(),
          });
        }
      }
      return deps.store.updateJobAfterLease({
        jobId: job.id,
        leaseToken,
        status: "posting_kickoff",
        now: now(),
      });
    }

    if (job.status === "posting_kickoff") {
      const groupId = job.createdGroupId ?? findResult(results, "created_group")?.groupConversationId;
      if (!groupId) {
        return deps.store.updateJobAfterLease({
          jobId: job.id,
          leaseToken,
          status: "failed",
          errorSummary: "Cannot post kickoff before group creation succeeds.",
          now: now(),
        });
      }
      const existingKickoff = findResult(results, "kickoff_post");
      if (existingKickoff?.result === "posted") {
        return deps.store.updateJobAfterLease({ jobId: job.id, leaseToken, status: "completed", now: now() });
      }
      const kickoffText = provisionedGroupKickoffText(plan, results);
      try {
        const message: ChatMessage = await sendMessageFn(
          input.sessionToken,
          input.actorId,
          groupId,
          kickoffText,
          { source: "group_provisioning", job_id: job.id, passive: true },
          "text",
          idempotencyKey(job, "kickoff", groupId),
        );
        const result: GroupProvisioningStepResult = {
          kind: "kickoff_post",
          stepId: "kickoff:post",
          groupConversationId: groupId,
          messageId: message.id,
          kickoffText,
          result: "posted",
        };
        return deps.store.updateJobAfterLease({
          jobId: job.id,
          leaseToken,
          status: "completed",
          stepResultsJson: stepState([...results, result], []),
          now: now(),
        });
      } catch (error) {
        const issue = issueFromError("kickoff_post_failed", error, { recoverable: true });
        return deps.store.updateJobAfterLease({
          jobId: job.id,
          leaseToken,
          status: "completed_with_warning",
          stepResultsJson: stepState([
            ...results,
            {
              kind: "kickoff_post",
              stepId: "kickoff:post:warning",
              groupConversationId: groupId,
              kickoffText,
              result: "warning",
              issue,
            },
            residualAssetsForJob(job),
          ], [issue]),
          errorSummary: errorMessage(error),
          now: now(),
        });
      }
    }

    return job;
  }

  async function runRolePlan(
    job: GroupProvisioningJob,
    plan: GroupLaunchPlanContract,
    rolePlan: ContractRoleSlotPlan,
    input: RunGroupProvisioningJobInput,
    leaseToken: string,
  ): Promise<GroupProvisioningStepResult> {
    if (rolePlan.action === "skip") {
      return {
        kind: "skipped_optional_role",
        stepId: `role:skip:${rolePlan.slotId}`,
        roleSlotId: rolePlan.slotId,
        roleTemplateId: rolePlan.roleTemplateId,
        reason: rolePlan.reason === "recovery_choice" ? "recovery_choice" : "user_choice",
      };
    }
    if (rolePlan.action === "reuse") {
      return {
        kind: "reused_agent",
        stepId: `role:reuse:${rolePlan.slotId}`,
        roleSlotId: rolePlan.slotId,
        agentId: rolePlan.existingAgentId,
        agentName: rolePlan.displayName?.trim() || rolePlan.existingAgentId,
        roleTemplateId: rolePlan.roleTemplateId,
        roleTemplateVersion: rolePlan.roleTemplateVersion,
      };
    }

    const groupTemplate = requireGroupTemplate(plan.groupTemplateId);
    const roleTemplate = requireRoleTemplate(rolePlan.roleTemplateId);
    const roleSlot = groupTemplate.roleSlots.find((slot) => slot.id === rolePlan.slotId);
    const rendered = renderRoleInstructions({
      roleTemplate,
      roleSlot,
      setupInputs: rolePlan.setupInputs,
      groupMission: plan.mission,
      outputExpectations: roleTemplate.outputContract.summary,
    });
    const body: ProvisionRequestBody = {
      name: rolePlan.agentName,
      driver_type: rolePlan.driver,
      harness_account_id: rolePlan.harnessAccountId,
      model: rolePlan.model,
      workspace_id: job.companyId,
      workspace_path: rolePlan.workspacePath,
      skill_paths: rolePlan.selectedSkills,
      ...(rolePlan.driver === "webhook_agent"
        ? {
            webhook_url: rolePlan.webhookUrl,
          }
        : { instructions: rendered.markdown }),
      template_metadata: {
        mode: "role_template",
        roleTemplateId: rolePlan.roleTemplateId,
        roleTemplateVersion: rolePlan.roleTemplateVersion,
        instructionStatus: rolePlan.instructionStatus,
        setupSummary: rolePlan.setupInputs,
        selectedSkills: rolePlan.selectedSkills,
        workspaceMode: rolePlan.workspaceMode,
        originatingGroupProvisioningJobId: job.id,
        originatingRoleSlotId: rolePlan.slotId,
      },
    };
    const response = await provisionAgentFn({
      sessionToken: input.sessionToken,
      actorId: input.actorId,
      body,
      jobContext: {
        jobId: job.id,
        roleSlotId: rolePlan.slotId,
        idempotencyKey: idempotencyKey(job, "agent", rolePlan.slotId),
        allowWebhookRegistrationReplayWithoutOutput: rolePlan.driver === "webhook_agent",
      },
      roleTemplateProvenance: {
        roleTemplateId: rolePlan.roleTemplateId,
        roleTemplateVersion: rolePlan.roleTemplateVersion,
        instructionStatus: rolePlan.instructionStatus,
        setupSummary: rolePlan.setupInputs as unknown as JsonValue,
        workspaceMode: rolePlan.workspaceMode,
        selectedSkills: rolePlan.selectedSkills,
      },
      stepRecorder: agentProvisioningStepRecorder(job.id, leaseToken),
      provenanceWriter: defaultRoleTemplateProvenanceWriter,
    });
    return {
      kind: "created_agent",
      stepId: `role:create:${rolePlan.slotId}`,
      roleSlotId: rolePlan.slotId,
      agentId: response.agent.id,
      agentName: response.agent.name,
      driver: rolePlan.driver,
      roleTemplateId: rolePlan.roleTemplateId,
      roleTemplateVersion: rolePlan.roleTemplateVersion,
      instructionStatus: rolePlan.instructionStatus,
      workspaceMode: rolePlan.workspaceMode,
    };
  }

  function agentProvisioningStepRecorder(
    jobId: string,
    leaseToken: string,
  ): AgentProvisioningStepRecorder {
    return {
      async readStep(input) {
        const latest = await deps.store.getJob(jobId);
        if (!latest) return null;
        return (readProvisioningState(latest).agentSteps[input.key]?.output ?? null) as never;
      },
      async recordStep(record) {
        const latest = await deps.store.getJob(jobId);
        if (!latest) throw new Error(`group provisioning job not found: ${jobId}`);
        const state = readProvisioningState(latest);
        await deps.store.updateJobAfterLease({
          jobId,
          leaseToken,
          stepResultsJson: stepState(
            state.results,
            state.issues,
            {
              ...state.agentSteps,
              [record.key]: record,
            },
          ),
          now: now(),
        });
      },
    };
  }

  async function persistGroupMetadata(
    job: GroupProvisioningJob,
    plan: GroupLaunchPlanContract,
    groupConversationId: string,
    results: GroupProvisioningStepResult[],
  ): Promise<void> {
    const template = requireGroupTemplate(plan.groupTemplateId);
    const runtimeWorkflow = workflowForAssignableRoleSlots(
      plan.workflow ?? workflowFromGroupTemplate(template),
      addedMemberRoleSlotIds(results),
    );
    await deps.store.insertGroupTemplateInstance({
      groupConversationId,
      groupTemplateId: template.id,
      groupTemplateVersion: template.version,
      mission: plan.mission,
      workflow: {
        // Persist only runtime-relevant workflow fields plus current assignable role defaults.
        steps: template.workflow.steps,
        description: template.workflow.description,
        ...runtimeWorkflow,
      },
      kickoffText: plan.kickoffText,
      outputContract: template.outputContract,
      originatingJobId: job.id,
    });
    for (const roleSlotId of skippedRoleSlotIds(plan, results)) {
      const rolePlan = plan.rolePlans.find((item) => item.slotId === roleSlotId);
      if (!rolePlan) continue;
      const slot = template.roleSlots.find((candidate) => candidate.id === rolePlan.slotId);
      await deps.store.insertRoleAssignment({
        id: idempotencyKey(job, "assignment", rolePlan.slotId),
        groupConversationId,
        slotId: rolePlan.slotId,
        required: slot?.required ?? false,
        action: "skipped",
        roleTemplateId: rolePlan.roleTemplateId,
        roleTemplateVersion: rolePlan.roleTemplateVersion,
        originatingJobId: job.id,
      });
    }
  }

  async function persistRoleAssignment(
    job: GroupProvisioningJob,
    plan: GroupLaunchPlanContract,
    groupConversationId: string,
    member: MemberAgentResult,
  ): Promise<void> {
    const template = requireGroupTemplate(plan.groupTemplateId);
    const slot = template.roleSlots.find((candidate) => candidate.id === member.roleSlotId);
    await deps.store.insertRoleAssignment({
      id: idempotencyKey(job, "assignment", member.roleSlotId),
      groupConversationId,
      slotId: member.roleSlotId,
      required: slot?.required ?? false,
      action: member.action,
      agentPrincipalId: member.agentId,
      roleTemplateId: member.roleTemplateId,
      roleTemplateVersion: member.roleTemplateVersion,
      instructionStatus: member.instructionStatus,
      originatingJobId: job.id,
    });
  }

  function toJobContract(job: GroupProvisioningJob): GroupProvisioningJobContract {
    const results = readStepResults(job);
    const plan = planWithAssignableWorkflowDefaults(assertPlan(job.planJson), results);
    const issues = readIssues(job);
    const statusContract = GROUP_PROVISIONING_STATUS_CONTRACT[job.status];
    return {
      id: job.id,
      status: job.status,
      companyId: job.companyId,
      requestedBy: job.requestedBy,
      idempotencyKey: job.idempotencyKey,
      plan,
      progressSteps: progressStepsFor(job, plan, results, issues),
      stepResults: results,
      issues,
      recoveryChoices: recoveryChoicesFor(job.status, issues),
      allowedUiActions: statusContract.uiActions,
      allowedBackendActions: statusContract.backendActions,
      createdAgentIds: createdAgentIdsFromResults(results),
      reusedAgentIds: reusedAgentIdsFromResults(results),
      ...(job.createdGroupId ? { createdGroupId: job.createdGroupId } : {}),
      ...(job.errorSummary ? { errorSummary: job.errorSummary } : {}),
      createdAt: iso(job.createdAt),
      updatedAt: iso(job.updatedAt),
      ...(job.completedAt ? { completedAt: iso(job.completedAt) } : {}),
      ...(job.canceledAt ? { canceledAt: iso(job.canceledAt) } : {}),
    };
  }

  async function validatePlanForJob(input: {
    plan: GroupLaunchPlanContract;
    companyId: string;
    sessionToken: string;
    currentJobId?: string;
  }): Promise<{ plan: GroupLaunchPlanContract; issues: GroupProvisioningIssue[] }> {
    const driverAvailability = await loadDriverAvailabilityFn();
    const reusedAgentIds = reusedAgentIdsFromPlan(input.plan);
    if (reusedAgentIds.length === 0) {
      const plan = planWithAssignableWorkflowDefaults(input.plan);
      return {
        plan,
        issues: validatePlanForContract(plan, input.companyId, [], driverAvailability),
      };
    }

    let candidates: ExistingAgentCandidate[] = [];
    const lookupIssues: GroupProvisioningIssue[] = [];
    try {
      candidates = await loadExistingAgentCandidatesFn({
        sessionToken: input.sessionToken,
        companyId: input.companyId,
        agentIds: reusedAgentIds,
      });
    } catch (error) {
      lookupIssues.push(...reusedAgentIds.map((agentId) => ({
        severity: "error" as const,
        code: "existing_agent_lookup_failed",
        message: `Could not verify existing agent ${agentId}: ${errorMessage(error)}`,
        recoverable: true,
        agentId,
      })));
    }

    try {
      const activeJobs = await deps.store.listActiveJobsForAgentIds({
        companyId: input.companyId,
        agentIds: reusedAgentIds,
      });
      const activeByAgentId = new Map<string, string[]>();
      for (const activeJob of activeJobs) {
        if (activeJob.id === input.currentJobId) continue;
        for (const agentId of reusedAgentIds) {
          if (activeJob.involvedAgentIds.includes(agentId) || activeJob.createdAgentIds.includes(agentId)) {
            activeByAgentId.set(agentId, [...(activeByAgentId.get(agentId) ?? []), activeJob.id]);
          }
        }
      }
      candidates = candidates.map((candidate) => ({
        ...candidate,
        activeJobIds: activeByAgentId.get(candidate.principal.id) ?? candidate.activeJobIds,
      }));
    } catch (error) {
      lookupIssues.push(...reusedAgentIds.map((agentId) => ({
        severity: "error" as const,
        code: "active_job_lookup_failed",
        message: `Could not verify active provisioning conflicts for ${agentId}: ${errorMessage(error)}`,
        recoverable: true,
        agentId,
      })));
    }

    const plan = planWithAssignableWorkflowDefaults(planWithReusedAgentDisplayNames(input.plan, candidates));
    return {
      plan,
      issues: [
        ...validatePlanForContract(plan, input.companyId, candidates, driverAvailability).map(blockingReuseWarningsForRunner),
        ...lookupIssues,
      ],
    };
  }

  return { createJob, getJob, runJob, retryJob, cancelJob, toJobContract };
}

function blockingReuseWarningsForRunner(issue: GroupProvisioningIssue): GroupProvisioningIssue {
  if (issue.severity === "warning" && BLOCKING_REUSE_ISSUE_CODES.has(issue.code)) {
    return { ...issue, severity: "error", recoverable: true };
  }
  return issue;
}

function planWithAssignableWorkflowDefaults(
  plan: GroupLaunchPlanContract,
  results: GroupProvisioningStepResult[] = [],
): GroupLaunchPlanContract {
  if (!plan.workflow) return plan;
  const assignableRoleSlotIds = currentAssignableRoleSlotIds(plan, results);
  return {
    ...plan,
    workflow: workflowForAssignableRoleSlots(plan.workflow, assignableRoleSlotIds),
  };
}

function currentAssignableRoleSlotIds(
  plan: GroupLaunchPlanContract,
  results: GroupProvisioningStepResult[],
): Set<string> {
  if (results.some((result) => result.kind === "member_add")) {
    return addedMemberRoleSlotIds(results);
  }
  return new Set(
    plan.rolePlans
      .filter((rolePlan) => rolePlan.action !== "skip")
      .map((rolePlan) => rolePlan.slotId),
  );
}

function workflowForAssignableRoleSlots(
  workflow: GroupLaunchPlanWorkflow,
  assignableRoleSlotIds: Set<string>,
): GroupLaunchPlanWorkflow {
  return {
    ...(workflow.coordinatorRoleSlotId && assignableRoleSlotIds.has(workflow.coordinatorRoleSlotId)
      ? { coordinatorRoleSlotId: workflow.coordinatorRoleSlotId }
      : {}),
    participantRoleDefaults: Object.fromEntries(
      Object.entries(workflow.participantRoleDefaults)
        .filter(([roleSlotId]) => assignableRoleSlotIds.has(roleSlotId)),
    ),
  };
}

function workflowFromGroupTemplate(template: GroupTemplate): GroupLaunchPlanWorkflow {
  return {
    ...(template.workflow.coordinatorRoleSlotId ? { coordinatorRoleSlotId: template.workflow.coordinatorRoleSlotId } : {}),
    participantRoleDefaults: template.workflow.participantRoleDefaults ?? Object.fromEntries(
      template.roleSlots.map((slot) => [slot.id, slot.workflowRoleKeys ?? []]),
    ),
  };
}

function planWithReusedAgentDisplayNames(
  plan: GroupLaunchPlanContract,
  candidates: ExistingAgentCandidate[],
): GroupLaunchPlanContract {
  const displayNameByAgentId = new Map(
    candidates.map((candidate) => [
      candidate.principal.id,
      sanitizeKickoffMemberName(candidate.principal.name),
    ]),
  );
  return {
    ...plan,
    rolePlans: plan.rolePlans.map((rolePlan) => {
      if (rolePlan.action !== "reuse") return rolePlan;
      const displayName = displayNameByAgentId.get(rolePlan.existingAgentId) || rolePlan.displayName?.trim();
      return {
        ...rolePlan,
        ...(displayName ? { displayName } : {}),
      };
    }),
  };
}

function sanitizeKickoffMemberName(name: string): string {
  return name
    .replace(/[\p{Cc}\p{Cf}]+/gu, " ")
    .replace(/@+/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

async function defaultEnableRoutingPolicy(input: {
  sessionToken: string;
  actorId: string;
  conversationId: string;
  idempotencyKey: string;
  coordinatorAgentId?: string;
}): Promise<void> {
  void input.actorId;
  void input.idempotencyKey;
  await upsertRuntimePolicy(input.sessionToken, input.conversationId, {
    allow_agent_to_agent: true,
    auto_mode: "mentioned_only",
    ...(input.coordinatorAgentId
      ? {
          default_coordinator_agent_id: input.coordinatorAgentId,
          untagged_human_mode: "coordinator_only",
        }
      : {}),
  });
}

async function defaultLoadExistingAgentCandidates(input: {
  sessionToken: string;
  companyId: string;
  agentIds: string[];
}): Promise<ExistingAgentCandidate[]> {
  if (input.agentIds.length === 0) return [];
  const expectedIds = new Set(input.agentIds);
  const [snapshot, runtimeBindings] = await Promise.all([
    fetchConsoleSnapshot(input.sessionToken),
    fetchRuntimeBindings(input.sessionToken),
  ]);
  const runtimeBindingByAgentId = new Map(
    runtimeBindings.map((binding) => [binding.agent_principal_id, binding]),
  );
  return snapshot.agents
    .filter((agent) => expectedIds.has(agent.id))
    .map((agent) => ({
      principal: agent,
      runtimeBinding: runtimeBindingByAgentId.get(agent.id) ?? null,
      companyId: agent.workspace_id,
      workspaceId: agent.workspace_id,
    }));
}

function validatePlanForContract(
  plan: GroupLaunchPlanContract,
  companyId: string,
  existingAgents: ExistingAgentCandidate[],
  driverAvailability: DriverAvailabilityItem[] = [],
): GroupProvisioningIssue[] {
  const validation = validateGroupLaunchPlan({
    groupTemplateId: plan.groupTemplateId,
    groupTemplateVersion: plan.groupTemplateVersion,
    groupName: plan.groupName,
    mission: plan.mission,
    companyId,
    workspaceId: companyId,
    existingAgents,
    driverAvailability,
    rolePlans: plan.rolePlans.map((rolePlan) => {
      if (rolePlan.action === "create") {
        return {
          slotId: rolePlan.slotId,
          action: "create" as const,
          agentName: rolePlan.agentName,
          setupInputs: rolePlan.setupInputs,
          userOverrideDriver: rolePlan.driver,
          instructions: "Role instructions will be rendered from the selected template during provisioning.",
          workspacePath: rolePlan.workspacePath,
          skillPaths: rolePlan.selectedSkills,
          webhookUrl: rolePlan.webhookUrl,
        };
      }
      if (rolePlan.action === "reuse") {
        return {
          slotId: rolePlan.slotId,
          action: "reuse" as const,
          existingAgentId: rolePlan.existingAgentId,
        };
      }
      return { slotId: rolePlan.slotId, action: "skip" as const };
    }),
  });
  return [...validation.errors, ...validation.warnings]
    .map((item) => ({
      severity: item.severity,
      code: item.code,
      message: item.message,
      recoverable: item.severity === "error",
      ...(item.field ? { field: item.field } : {}),
      ...(item.slotId ? { roleSlotId: item.slotId } : {}),
    }));
}

function progressStepsFor(
  job: GroupProvisioningJob,
  plan: GroupLaunchPlanContract,
  results: GroupProvisioningStepResult[],
  issues: GroupProvisioningIssue[],
): ProgressStep[] {
  const steps: ProgressStep[] = [{
    id: "validation",
    kind: "validation",
    label: "Validate launch plan",
    status: validationProgressStatus(job.status, issues),
    ...(issues.length ? { issues } : {}),
  }];
  for (const rolePlan of plan.rolePlans) {
    steps.push({
      id: `role:${rolePlan.slotId}`,
      kind: "agent_creation",
      label: rolePlan.action === "create" ? `Create ${rolePlan.agentName}` : `Prepare ${rolePlan.slotId}`,
      roleSlotId: rolePlan.slotId,
      status: roleProgressStatus(job.status, results, rolePlan),
    });
  }
  steps.push({
    id: "group:create",
    kind: "group_creation",
    label: "Create group",
    status: findResult(results, "created_group") ? "succeeded" : statusToPendingOrRunning(job.status, "creating_group"),
  });
  for (const member of memberAgentResults(results)) {
    const memberResult = findMemberAddResult(results, member.roleSlotId);
    steps.push({
      id: `member:add:${member.roleSlotId}`,
      kind: "member_add",
      label: `Add ${member.agentId}`,
      roleSlotId: member.roleSlotId,
      status: memberResult?.result === "added"
        ? "succeeded"
        : memberResult?.result === "skipped"
          ? "skipped"
          : memberResult?.result === "failed"
            ? "failed"
            : statusToPendingOrRunning(job.status, "adding_members"),
    });
  }
  steps.push({
    id: "routing-policy:enable",
    kind: "routing_policy_update",
    label: "Enable routing policy",
    status: findResult(results, "routing_policy_update") ? "succeeded" : statusToPendingOrRunning(job.status, "adding_members"),
  });
  steps.push({
    id: "kickoff:post",
    kind: "kickoff_post",
    label: "Post kickoff",
    status: kickoffProgressStatus(results, job.status),
  });
  if (job.status === "canceled" || results.some((result) => result.kind === "cleanup")) {
    steps.push({
      id: "cleanup:cancel",
      kind: "cleanup",
      label: "Record cleanup choice",
      status: job.status === "canceled" ? "canceled" : "pending",
    });
  }
  return steps;
}

function validationProgressStatus(status: GroupProvisioningJobStatus, issues: GroupProvisioningIssue[]): ProgressStep["status"] {
  if (status === "validating") return "running";
  if (status === "failed_validation") return "failed";
  if (issues.some((issue) => issue.severity === "warning")) return "warning";
  return "succeeded";
}

function roleProgressStatus(
  status: GroupProvisioningJobStatus,
  results: GroupProvisioningStepResult[],
  rolePlan: ContractRoleSlotPlan,
): ProgressStep["status"] {
  const result = results.find((item) =>
    (item.kind === "created_agent" || item.kind === "reused_agent" || item.kind === "skipped_optional_role")
    && item.roleSlotId === rolePlan.slotId,
  );
  if (result?.kind === "skipped_optional_role") return "skipped";
  if (result) return "succeeded";
  return statusToPendingOrRunning(status, "creating_agents");
}

function statusToPendingOrRunning(current: GroupProvisioningJobStatus, runningStatus: GroupProvisioningJobStatus): ProgressStep["status"] {
  if (current === runningStatus) return "running";
  if (current === "failed" || current === "partial_failure" || current === "failed_validation") return "failed";
  if (current === "canceled") return "canceled";
  if (current === "completed" || current === "completed_with_warning") return "succeeded";
  return "pending";
}

function kickoffProgressStatus(
  results: GroupProvisioningStepResult[],
  status: GroupProvisioningJobStatus,
): ProgressStep["status"] {
  const result = findLatestResult(results, "kickoff_post");
  if (result?.result === "posted") return "succeeded";
  if (result?.result === "warning") return "warning";
  if (result?.result === "failed") return "failed";
  if (result?.result === "skipped") return "skipped";
  return statusToPendingOrRunning(status, "posting_kickoff");
}

function recoveryChoicesFor(status: GroupProvisioningJobStatus, issues: GroupProvisioningIssue[]): RecoveryChoice[] {
  if (status === "failed_validation") {
    return [
      { id: "edit_plan", label: "Edit plan", description: "Return to plan editing.", nextStatus: "validating" },
      { id: "retry_validation", label: "Retry validation", description: "Validate the same plan again.", nextStatus: "validating" },
      { id: "cancel", label: "Cancel", description: "Stop this provisioning job.", destructive: true },
    ];
  }
  if (status === "partial_failure" || status === "failed") {
    const code = issues.at(-1)?.code ?? "";
    const issue = issues.at(-1);
    if (code.includes("group_creation")) {
      return [
        { id: "retry_group_creation", label: "Retry group creation", description: "Try creating the group conversation again.", nextStatus: "creating_group" },
        { id: "soft_delete_generated_agents", label: "Soft-delete generated agents", description: "Disable generated agents created only for this job.", destructive: true, nextStatus: "rolled_back" },
        { id: "cancel", label: "Cancel", description: "Stop this provisioning job.", destructive: true },
      ];
    }
    if (code.includes("member_add") || code.includes("routing_policy")) {
      const choices: RecoveryChoice[] = [
        { id: "retry_member_add", label: "Retry member add", description: "Try adding the missing group member again.", roleSlotId: issue?.roleSlotId, nextStatus: "adding_members" },
        { id: "replace_agent", label: "Replace agent", description: "Choose another agent for the missing role.", roleSlotId: issue?.roleSlotId, nextStatus: "creating_agents" },
        { id: "manual_invite", label: "Manual invite", description: "Leave the missing member for a manual invite.", roleSlotId: issue?.roleSlotId, nextStatus: "adding_members" },
        { id: "cancel", label: "Cancel", description: "Stop this provisioning job.", destructive: true },
      ];
      if (code.includes("optional_member_add")) {
        choices.splice(2, 0, {
          id: "skip_optional_role",
          label: "Skip optional member",
          description: "Keep the group and continue without this optional member.",
          roleSlotId: issue?.roleSlotId,
          nextStatus: "adding_members",
        });
      }
      return choices;
    }
    if (code.includes("optional_agent_creation")) {
      return [
        { id: "retry_agent_creation", label: "Retry optional agent", description: "Try creating the optional agent again.", roleSlotId: issue?.roleSlotId, nextStatus: "creating_agents" },
        { id: "skip_optional_role", label: "Skip optional role", description: "Continue without this optional role.", roleSlotId: issue?.roleSlotId, nextStatus: "creating_agents" },
        { id: "edit_plan", label: "Edit setup", description: "Return to setup and adjust this role.", roleSlotId: issue?.roleSlotId, nextStatus: "validating" },
        { id: "cancel", label: "Cancel", description: "Stop this provisioning job.", destructive: true },
      ];
    }
    const retryId = code.includes("required_agent_creation") ? "retry_agent_creation" : "retry_agent_creation";
    return [
      { id: retryId, label: "Retry required agent", description: "Retry creating the required agent.", roleSlotId: issue?.roleSlotId, nextStatus: retryStatusFor(status, retryId) },
      { id: "edit_plan", label: "Edit setup", description: "Return to setup and adjust this role.", roleSlotId: issue?.roleSlotId, nextStatus: "validating" },
      { id: "soft_delete_generated_agents", label: "Soft-delete generated agents", description: "Disable generated agents created only for this job.", destructive: true, nextStatus: "rolled_back" },
      { id: "cancel", label: "Keep generated agents and cancel", description: "Stop this provisioning job and preserve already generated agents.", destructive: true },
    ];
  }
  if (status === "completed_with_warning") {
    return [
      { id: "retry_kickoff", label: "Retry kickoff", description: "Try posting the kickoff message again.", nextStatus: "posting_kickoff" },
      { id: "enter_group", label: "Enter group", description: "Open the group and continue without reposting kickoff." },
    ];
  }
  return [];
}

function retryStatusFor(status: GroupProvisioningJobStatus, choice: ProvisioningJobRetryRequest["choice"]): GroupProvisioningJobStatus {
  if (choice === "retry_validation" || choice === "edit_plan") return "validating";
  if (choice === "retry_agent_creation" || choice === "skip_optional_role" || choice === "replace_agent") return "creating_agents";
  if (choice === "retry_group_creation") return "creating_group";
  if (choice === "retry_member_add" || choice === "manual_invite") return "adding_members";
  if (choice === "retry_kickoff" || status === "completed_with_warning") return "posting_kickoff";
  return "validating";
}

function readStepResults(job: GroupProvisioningJob): GroupProvisioningStepResult[] {
  return readProvisioningState(job).results;
}

function readIssues(job: GroupProvisioningJob): GroupProvisioningIssue[] {
  return readProvisioningState(job).issues;
}

function readProvisioningState(job: GroupProvisioningJob): ProvisioningStepState {
  const value = job.stepResultsJson;
  if (Array.isArray(value)) {
    return {
      results: value as unknown as GroupProvisioningStepResult[],
      issues: [],
      agentSteps: {},
    };
  }
  if (!isRecord(value)) {
    return { results: [], issues: [], agentSteps: {} };
  }
  const agentSteps = isRecord(value.agentSteps)
    ? value.agentSteps as unknown as AgentProvisioningStepMap
    : {};
  return {
    results: Array.isArray(value.results) ? value.results as unknown as GroupProvisioningStepResult[] : [],
    issues: Array.isArray(value.issues) ? value.issues as unknown as GroupProvisioningIssue[] : [],
    agentSteps,
  };
}

function stepState(
  results: GroupProvisioningStepResult[],
  issues: GroupProvisioningIssue[],
  agentSteps: AgentProvisioningStepMap = {},
): JsonValue {
  return {
    results: results as unknown as JsonValue[],
    issues: issues as unknown as JsonValue[],
    agentSteps: agentSteps as unknown as JsonValue,
  };
}

function assertPlan(value: JsonValue): GroupLaunchPlanContract {
  if (!isRecord(value) || !Array.isArray(value.rolePlans)) {
    throw new Error("group provisioning job has an invalid launch plan");
  }
  return value as unknown as GroupLaunchPlanContract;
}

function findResult<K extends GroupProvisioningStepResult["kind"]>(
  results: GroupProvisioningStepResult[],
  kind: K,
): Extract<GroupProvisioningStepResult, { kind: K }> | undefined {
  return results.find((result) => result.kind === kind) as Extract<GroupProvisioningStepResult, { kind: K }> | undefined;
}

function findLatestResult<K extends GroupProvisioningStepResult["kind"]>(
  results: GroupProvisioningStepResult[],
  kind: K,
): Extract<GroupProvisioningStepResult, { kind: K }> | undefined {
  for (let index = results.length - 1; index >= 0; index -= 1) {
    const result = results[index];
    if (result.kind === kind) return result as Extract<GroupProvisioningStepResult, { kind: K }>;
  }
  return undefined;
}

function hasRoleResult(results: GroupProvisioningStepResult[], roleSlotId: string): boolean {
  return results.some((result) =>
    (result.kind === "created_agent" || result.kind === "reused_agent" || result.kind === "skipped_optional_role")
    && result.roleSlotId === roleSlotId,
  );
}

function hasMemberAddResult(results: GroupProvisioningStepResult[], roleSlotId: string): boolean {
  return results.some((result) =>
    result.kind === "member_add"
    && result.roleSlotId === roleSlotId
    && (result.result === "added" || result.result === "skipped")
  );
}

function findMemberAddResult(
  results: GroupProvisioningStepResult[],
  roleSlotId: string,
): Extract<GroupProvisioningStepResult, { kind: "member_add" }> | undefined {
  for (let index = results.length - 1; index >= 0; index -= 1) {
    const result = results[index];
    if (result.kind === "member_add" && result.roleSlotId === roleSlotId) return result;
  }
  return undefined;
}

function addedMemberRoleSlotIds(results: GroupProvisioningStepResult[]): Set<string> {
  return new Set(
    results.flatMap((result) =>
      result.kind === "member_add" && result.result === "added" ? [result.roleSlotId] : []
    ),
  );
}

function skippedRoleSlotIds(
  plan: GroupLaunchPlanContract,
  results: GroupProvisioningStepResult[],
): Set<string> {
  return new Set([
    ...plan.rolePlans
      .filter((rolePlan) => rolePlan.action === "skip")
      .map((rolePlan) => rolePlan.slotId),
    ...results.flatMap((result) => {
      if (result.kind === "skipped_optional_role") return [result.roleSlotId];
      if (result.kind === "member_add" && result.result === "skipped") return [result.roleSlotId];
      return [];
    }),
  ]);
}

function memberAgentResults(results: GroupProvisioningStepResult[]): MemberAgentResult[] {
  return results.flatMap<MemberAgentResult>((result) => {
    if (result.kind === "created_agent") {
      return [{
        action: "created" as const,
        roleSlotId: result.roleSlotId,
        agentId: result.agentId,
        agentName: result.agentName,
        roleTemplateId: result.roleTemplateId,
        roleTemplateVersion: result.roleTemplateVersion,
        instructionStatus: result.instructionStatus,
      }];
    }
    if (result.kind === "reused_agent") {
      return [{
        action: "reused" as const,
        roleSlotId: result.roleSlotId,
        agentId: result.agentId,
        agentName: result.agentName,
        roleTemplateId: result.roleTemplateId,
        roleTemplateVersion: result.roleTemplateVersion,
        instructionStatus: "template_default" as const,
      }];
    }
    return [];
  });
}

function provisionedGroupKickoffText(
  plan: GroupLaunchPlanContract,
  results: GroupProvisioningStepResult[],
): string {
  const template = requireGroupTemplate(plan.groupTemplateId);
  const addedMembers = new Set(
    results.flatMap((result) =>
      result.kind === "member_add" && result.result === "added" ? [result.roleSlotId] : []
    ),
  );
  const membersBySlot = new Map(
    memberAgentResults(results)
      .filter((member) => addedMembers.has(member.roleSlotId))
      .map((member) => [member.roleSlotId, member]),
  );
  const members: GroupKickoffMember[] = template.roleSlots.flatMap((slot) => {
    const member = membersBySlot.get(slot.id);
    if (!member) return [];
    const name = sanitizeKickoffMemberName(member.agentName || member.agentId) || member.agentId;
    return [{ name, roleLabel: slot.label }];
  });
  return renderGroupKickoff(template, plan.mission, { members });
}

function coordinatorRoutingPolicyInput(
  plan: GroupLaunchPlanContract,
  results: GroupProvisioningStepResult[],
): { coordinatorAgentId?: string } {
  const coordinatorRoleSlotId = plan.workflow?.coordinatorRoleSlotId;
  if (!coordinatorRoleSlotId) return {};
  if (findMemberAddResult(results, coordinatorRoleSlotId)?.result !== "added") return {};
  const coordinator = memberAgentResults(results).find((member) => member.roleSlotId === coordinatorRoleSlotId);
  return coordinator ? { coordinatorAgentId: coordinator.agentId } : {};
}

function involvedAgentIdsFromPlan(plan: GroupLaunchPlanContract): string[] {
  return plan.rolePlans.flatMap((rolePlan) => rolePlan.action === "reuse" ? [rolePlan.existingAgentId] : []);
}

function reusedAgentIdsFromPlan(plan: GroupLaunchPlanContract): string[] {
  return [...new Set(involvedAgentIdsFromPlan(plan))];
}

function involvedAgentIdsFromResults(results: GroupProvisioningStepResult[], plan: GroupLaunchPlanContract): string[] {
  return [...new Set([...involvedAgentIdsFromPlan(plan), ...createdAgentIdsFromResults(results)])];
}

function createdAgentIdsFromResults(results: GroupProvisioningStepResult[]): string[] {
  return results.flatMap((result) => result.kind === "created_agent" ? [result.agentId] : []);
}

function reusedAgentIdsFromResults(results: GroupProvisioningStepResult[]): string[] {
  return results.flatMap((result) => result.kind === "reused_agent" ? [result.agentId] : []);
}

function generatedAgentIdsFromJob(job: GroupProvisioningJob): string[] {
  const state = readProvisioningState(job);
  const reused = new Set(reusedAgentIdsFromResults(state.results));
  const ids = new Set([...job.createdAgentIds, ...createdAgentIdsFromResults(state.results)]);
  for (const record of Object.values(state.agentSteps)) {
    for (const candidate of agentIdsFromStepOutput(record.output)) {
      ids.add(candidate);
    }
  }
  return [...ids].filter((agentId) => !reused.has(agentId));
}

async function generatedAgentIdsForCleanup(
  job: GroupProvisioningJob,
  loadGeneratedAgentCleanupCandidatesFn: GroupProvisioningRunnerDeps["loadGeneratedAgentCleanupCandidates"],
): Promise<string[]> {
  const ids = new Set(generatedAgentIdsFromJob(job));
  if (!loadGeneratedAgentCleanupCandidatesFn) return [...ids];

  const plan = assertPlan(job.planJson);
  const recordedIds = new Set(ids);
  const recordedNames = new Set(
    readStepResults(job)
      .flatMap((result) => result.kind === "created_agent" ? [normalizeAgentName(result.agentName)] : []),
  );
  const missingNames = [
    ...new Set(plan.rolePlans
      .filter((rolePlan) => rolePlan.action === "create")
      .map((rolePlan) => rolePlan.agentName)
      .filter((name) => !recordedNames.has(normalizeAgentName(name)))),
  ];
  if (missingNames.length === 0) return [...ids];

  const candidates = await loadGeneratedAgentCleanupCandidatesFn({
    companyId: job.companyId,
    agentNames: missingNames,
  });
  const requestedNames = new Set(missingNames.map(normalizeAgentName));
  const jobCreatedAt = Date.parse(iso(job.createdAt));
  for (const candidate of candidates) {
    const principal = candidate.principal;
    const createdAt = Date.parse(principal.created_at);
    if (recordedIds.has(principal.id)) continue;
    if (candidate.companyId !== job.companyId && candidate.workspaceId !== job.companyId) continue;
    if (principal.disabled) continue;
    if (!requestedNames.has(normalizeAgentName(principal.name))) continue;
    if (!Number.isFinite(createdAt) || createdAt < jobCreatedAt) continue;
    ids.add(principal.id);
  }
  return [...ids];
}

function agentIdsFromStepOutput(output: unknown): string[] {
  if (!isRecord(output)) return [];
  const ids: string[] = [];
  if (isRecord(output.agent) && typeof output.agent.id === "string") ids.push(output.agent.id);
  if (isRecord(output.principal) && typeof output.principal.id === "string") ids.push(output.principal.id);
  return ids;
}

function normalizeAgentName(name: string): string {
  return name.trim().toLowerCase();
}

function residualAssetsForJob(job: GroupProvisioningJob): Extract<GroupProvisioningStepResult, { kind: "residual_assets" }> {
  const plan = assertPlan(job.planJson);
  const results = readStepResults(job);
  const customWorkspacePathsPreserved = plan.rolePlans.flatMap((rolePlan) =>
    rolePlan.action === "create" && rolePlan.workspaceMode === "custom" && rolePlan.workspacePath
      ? [rolePlan.workspacePath]
      : []
  );
  return {
    kind: "residual_assets",
    stepId: `residual-assets:${job.id}`,
    ...(job.createdGroupId ? { groupConversationId: job.createdGroupId } : {}),
    generatedAgentIds: generatedAgentIdsFromJob(job),
    reusedAgentIds: reusedAgentIdsFromResults(results),
    customWorkspacePathsPreserved,
    note: "Generated agents are soft-disabled only through principal disable; reused agents and custom workspace paths are preserved.",
  };
}

function rolePlanIsRequired(plan: GroupLaunchPlanContract, roleSlotId: string): boolean {
  const template = getGroupTemplate(plan.groupTemplateId);
  return template?.roleSlots.find((slot) => slot.id === roleSlotId)?.required ?? true;
}

function issueFromError(
  code: string,
  error: unknown,
  options: { recoverable: boolean; roleSlotId?: string; roleTemplateId?: string; agentId?: string },
): GroupProvisioningIssue {
  const message = error instanceof AgentProvisioningError
    ? error.detail.message
    : errorMessage(error);
  return {
    severity: "error",
    code,
    message,
    recoverable: options.recoverable,
    ...(options.roleSlotId ? { roleSlotId: options.roleSlotId } : {}),
    ...(options.roleTemplateId ? { roleTemplateId: options.roleTemplateId } : {}),
    ...(options.agentId ? { agentId: options.agentId } : {}),
  };
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Provisioning step failed.";
}

function idempotencyKey(job: GroupProvisioningJob, step: string, subject: string): string {
  return `group-provisioning:${job.id}:${job.idempotencyKey}:${step}:${subject}`;
}

function requireGroupTemplate(id: string): GroupTemplate {
  const template = getGroupTemplate(id);
  if (!template) throw new Error(`Unknown group template ${id}`);
  return template;
}

function requireRoleTemplate(id: string): RoleTemplate {
  const template = getRoleTemplate(id);
  if (!template) throw new Error(`Unknown role template ${id}`);
  return template;
}

function iso(value: Date | string): string {
  return value instanceof Date ? value.toISOString() : value;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}
