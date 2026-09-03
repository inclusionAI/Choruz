"use client";

import { X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { trace } from "../../lib/api/choruz-trace";
import { Modal } from "../ui/modal";
import type { Principal, Conversation, RuntimeBindingInfo } from "../../lib/api/choruz-types";
import { Avatar } from "../ui/avatar";
import { isAgent } from "../../lib/api/principals";
import { apiFetch } from "../../lib/api/choruz-api";
import {
  applyGroupDefaultDriver,
  blockingGroupTemplateIssues,
  buildGroupReviewItems,
  buildProvisioningJobCreationRequest,
  cancelGroupProvisioningJob,
  createGroupProvisioningJob,
  createGroupTemplateDraft,
  createRoleDraftForSlot,
  groupTemplateIssues,
  groupTemplateOptions,
  isGroupProvisioningTerminal,
  readGroupProvisioningJob,
  retryGroupProvisioningJob,
  runGroupProvisioningJob,
  updateDraftMission,
  GROUP_TEMPLATE_DRIVER_IDS,
  type GroupTemplateDraft,
  type GroupTemplateRoleDraft,
  type GroupTemplateWorkspaceMode,
} from "../../lib/groups/create-group-template-flow";
import type {
  GroupProvisioningJobContract,
  RecoveryChoice,
} from "../../lib/groups/group-provisioning-contract";
import { groupProvisioningIssueClassName } from "../../lib/groups/group-provisioning-issue-display";
import {
  getGroupTemplate,
  getRoleTemplate,
  type DriverId,
} from "../../lib/groups/team-templates";
import { useDriverAvailability } from "../../hooks/use-driver-availability";
import { DriverSelect } from "../agents/driver-select";
import { PathPicker } from "../workspace/path-picker";
import { SetupInputField } from "./setup-input-field";
import { StepTabs } from "./step-tabs";
import { driverDisplayName } from "../../lib/drivers/driver-registry";
import { DriverModelPicker } from "../agents/driver-model-picker";
import { HarnessAccountPicker } from "../agents/harness-account-picker";

export function GroupProvisioningIssueList({ issues }: { issues: GroupProvisioningJobContract["issues"] }) {
  return (
    <>
      {issues.map((issue) => (
        <p key={`${issue.code}-${issue.message}`} className={groupProvisioningIssueClassName(issue)}>
          {issue.message}
        </p>
      ))}
    </>
  );
}

function shouldForgetProvisioningJob(job: GroupProvisioningJobContract): boolean {
  return ["completed", "rolled_back", "canceled"].includes(job.status);
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export type CreateGroupModalProps = {
  principalId: string;
  sessionToken: string;
  agents: Principal[];
  runtimeBindings: RuntimeBindingInfo[];
  activeCompanyId?: string | null;
  onClose: () => void;
  onCreated: (conversationId: string) => void;
  refreshSnapshot: () => Promise<void>;
  agentSkillsEnabled: boolean;
  /** The company's switch: when off, each new role uses the login its device already has. */
  multiHarnessAccounts: boolean;
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

const CREATE_GROUP_STEPS = [
  { id: "setup", label: "Setup" },
  { id: "review", label: "Review & Launch" },
  { id: "progress", label: "Progress" },
] as const;

export function CreateGroupModal({
  principalId,
  sessionToken,
  agents,
  runtimeBindings,
  activeCompanyId,
  onClose,
  onCreated,
  refreshSnapshot,
  agentSkillsEnabled,
  multiHarnessAccounts,
}: CreateGroupModalProps) {
  const groupNameRef = useRef<HTMLInputElement>(null);
  const [step, setStep] = useState<"setup" | "review" | "progress">("setup");
  const [selectedGroupTemplateId, setSelectedGroupTemplateId] = useState("");
  const [groupTemplateDraft, setGroupTemplateDraft] = useState<GroupTemplateDraft | null>(null);
  const [groupNameEdited, setGroupNameEdited] = useState(false);
  const [provisioningJob, setProvisioningJob] = useState<GroupProvisioningJobContract | null>(null);
  const [launchingGroup, setLaunchingGroup] = useState(false);
  const [launchError, setLaunchError] = useState<string | null>(null);
  const [groupName, setGroupName] = useState("");
  const [groupSelectedIds, setGroupSelectedIds] = useState<Set<string>>(
    new Set(),
  );
  const [groupSearchQuery, setGroupSearchQuery] = useState("");
  const [createGroupError, setCreateGroupError] = useState<string | null>(null);
  const {
    availability: driverAvailability,
    loaded: driverAvailabilityLoaded,
    error: driverAvailabilityError,
  } = useDriverAvailability();

  const groupCandidates = useMemo(() => {
    const result: {
      id: string;
      name: string;
      type: "agent" | "human";
      driver: string;
    }[] = [];
    for (const a of agents) {
      if (a.disabled) continue;
      // Only show agents belonging to the active company/workspace
      if (activeCompanyId && a.workspace_id !== activeCompanyId) continue;
      result.push({
        id: a.id,
        name: a.name,
        type: "agent",
        driver: "Agent",
      });
    }
    return result;
  }, [agents, activeCompanyId]);

  const filteredGroupCandidates = useMemo(() => {
    if (!groupSearchQuery.trim()) return groupCandidates;
    const q = groupSearchQuery.toLowerCase();
    return groupCandidates.filter(
      (c) =>
        c.name.toLowerCase().includes(q) || c.id.toLowerCase().includes(q),
      );
  }, [groupCandidates, groupSearchQuery]);

  const groupTemplates = useMemo(() => groupTemplateOptions(), []);
  const selectedGroupTemplate = useMemo(
    () => (selectedGroupTemplateId ? getGroupTemplate(selectedGroupTemplateId) ?? null : null),
    [selectedGroupTemplateId],
  );
  const templateIssues = useMemo(
    () =>
      groupTemplateDraft
        ? groupTemplateIssues({
            draft: groupTemplateDraft,
            existingAgents: agents,
            runtimeBindings,
            availability: driverAvailability,
          })
        : [],
    [agents, driverAvailability, groupTemplateDraft, runtimeBindings],
  );
  const blockingTemplateIssues = useMemo(
    () =>
      groupTemplateDraft
        ? blockingGroupTemplateIssues({
            draft: groupTemplateDraft,
            existingAgents: agents,
            runtimeBindings,
            availability: driverAvailability,
          })
        : [],
    [agents, driverAvailability, groupTemplateDraft, runtimeBindings],
  );
  const availabilityBlockingMessage = useMemo(() => {
    if (driverAvailabilityLoaded) return null;
    return driverAvailabilityError ?? "Checking driver availability…";
  }, [driverAvailabilityError, driverAvailabilityLoaded]);
  const reviewItems = useMemo(
    () => (groupTemplateDraft ? buildGroupReviewItems(groupTemplateDraft) : []),
    [groupTemplateDraft],
  );
  const activeJobStorageKey = useMemo(
    () => `choruz:group-provisioning-job:${activeCompanyId ?? "default"}`,
    [activeCompanyId],
  );

  const toggleGroupMember = useCallback((id: string) => {
    setGroupSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const handleGroupTemplateSelect = useCallback(
    (templateId: string) => {
      setLaunchError(null);
      setCreateGroupError(null);
      setProvisioningJob(null);
      setStep("setup");
      setSelectedGroupTemplateId(templateId);
      if (!templateId) {
        setGroupTemplateDraft(null);
        return;
      }
      const template = getGroupTemplate(templateId);
      if (!template) return;
      trace.event("template_group_template_selected", {
        groupTemplateId: template.id,
        groupTemplateVersion: template.version,
      });
      const draft = createGroupTemplateDraft({ groupTemplate: template });
      setGroupNameEdited(false);
      setGroupTemplateDraft(draft);
      setGroupName(draft.groupName);
    },
    [],
  );

  const updateGroupDraft = useCallback((updater: (draft: GroupTemplateDraft) => GroupTemplateDraft) => {
    setGroupTemplateDraft((current) => {
      if (!current) return current;
      const next = updater(current);
      setGroupName(next.groupName);
      setProvisioningJob(null);
      setLaunchError(null);
      return next;
    });
    setStep("setup");
  }, []);

  const updateRoleDraft = useCallback(
    (slotId: string, patch: Partial<GroupTemplateRoleDraft>) => {
      updateGroupDraft((draft) => ({
        ...draft,
        roleDrafts: draft.roleDrafts.map((role) =>
          role.slotId === slotId ? { ...role, ...patch } : role
        ),
      }));
    },
    [updateGroupDraft],
  );

  const resetRoleDraft = useCallback(
    (slotId: string) => {
      updateGroupDraft((draft) => {
        const template = getGroupTemplate(draft.groupTemplateId);
        const slot = template?.roleSlots.find((candidate) => candidate.id === slotId);
        if (!template || !slot) return draft;
        const nextRole = createRoleDraftForSlot({
          groupTemplate: template,
          roleSlot: slot,
          mission: draft.mission,
          groupDefaultDriver: draft.groupDefaultDriver,
        });
        return {
          ...draft,
          roleDrafts: draft.roleDrafts.map((role) => role.slotId === slotId ? { ...nextRole, action: "create" } : role),
        };
      });
    },
    [updateGroupDraft],
  );

  const advanceProvisioningJob = useCallback(
    async (jobId: string) => {
      setLaunchingGroup(true);
      setLaunchError(null);
      try {
        let current = await runGroupProvisioningJob(jobId, 3);
        setProvisioningJob(current);
        for (let index = 0; index < 40 && !isGroupProvisioningTerminal(current); index += 1) {
          current = await runGroupProvisioningJob(jobId, 3);
          setProvisioningJob(current);
        }
        if (shouldForgetProvisioningJob(current)) {
          window.localStorage.removeItem(activeJobStorageKey);
        }
        if (current.status === "completed" && current.createdGroupId) {
          await refreshSnapshot();
        }
      } catch (err) {
        setLaunchError(err instanceof Error ? err.message : "Failed to advance group launch.");
      } finally {
        setLaunchingGroup(false);
      }
    },
    [activeJobStorageKey, refreshSnapshot],
  );

  useEffect(() => {
    const jobId = window.localStorage.getItem(activeJobStorageKey);
    if (!jobId) return;
    let cancelled = false;
    void readGroupProvisioningJob(jobId)
      .then((job) => {
        if (cancelled) return;
        if (job.companyId !== activeCompanyId) {
          window.localStorage.removeItem(activeJobStorageKey);
          return;
        }
        setProvisioningJob(job);
        setStep("progress");
        if (shouldForgetProvisioningJob(job)) {
          window.localStorage.removeItem(activeJobStorageKey);
          return;
        }
        if (!isGroupProvisioningTerminal(job)) {
          void advanceProvisioningJob(job.id);
        }
      })
      .catch(() => {
        window.localStorage.removeItem(activeJobStorageKey);
      });
    return () => {
      cancelled = true;
    };
  }, [activeCompanyId, activeJobStorageKey, advanceProvisioningJob]);

  const handleLaunchTemplateGroup = useCallback(async () => {
    if (!groupTemplateDraft) return;
    const launchDraft = agentSkillsEnabled
      ? groupTemplateDraft
      : {
          ...groupTemplateDraft,
          roleDrafts: groupTemplateDraft.roleDrafts.map((role) => ({
            ...role,
            selectedSkills: [],
          })),
        };
    if (!activeCompanyId) {
      setLaunchError("Select a company before launching a group.");
      setStep("setup");
      return;
    }
    if (!driverAvailabilityLoaded) {
      setLaunchError(availabilityBlockingMessage ?? "Driver availability has not loaded.");
      setStep("setup");
      return;
    }
    const issues = blockingGroupTemplateIssues({
      draft: launchDraft,
      existingAgents: agents,
      runtimeBindings,
      availability: driverAvailability,
    });
    if (issues.length > 0) {
      setLaunchError(issues.map((issue) => issue.message).join(" "));
      setStep("setup");
      return;
    }
    setLaunchingGroup(true);
    setLaunchError(null);
    const span = trace.start("group_template_launch", {
      groupTemplateId: groupTemplateDraft.groupTemplateId,
      roleCount: groupTemplateDraft.roleDrafts.length,
    });
    try {
      const job = await createGroupProvisioningJob(buildProvisioningJobCreationRequest(launchDraft, activeCompanyId));
      setProvisioningJob(job);
      setStep("progress");
      window.localStorage.setItem(activeJobStorageKey, job.id);
      trace.event("template_group_launched", {
        groupTemplateId: groupTemplateDraft.groupTemplateId,
        groupTemplateVersion: groupTemplateDraft.groupTemplateVersion,
        jobId: job.id,
      });
      span.end({ status: 201, jobId: job.id });
      await advanceProvisioningJob(job.id);
    } catch (err) {
      span.end({ error: err instanceof Error ? err.message : String(err) });
      trace.event("template_group_launch_failed", {
        groupTemplateId: groupTemplateDraft.groupTemplateId,
        error: err instanceof Error ? err.message : String(err),
      });
      setLaunchError(err instanceof Error ? err.message : "Failed to launch group.");
    } finally {
      setLaunchingGroup(false);
    }
  }, [
    activeCompanyId,
    agentSkillsEnabled,
    activeJobStorageKey,
    advanceProvisioningJob,
    agents,
    driverAvailability,
    driverAvailabilityLoaded,
    groupTemplateDraft,
    availabilityBlockingMessage,
    runtimeBindings,
  ]);

  const handleRecoveryChoice = useCallback(
    async (choice: RecoveryChoice) => {
      if (!provisioningJob) return;
      trace.event("template_recovery_selected", {
        choiceId: choice.id,
        roleSlotId: choice.roleSlotId,
        jobStatus: provisioningJob.status,
      });
      if (choice.id.startsWith("retry_")) {
        trace.event("template_retry_selected", {
          choiceId: choice.id,
          roleSlotId: choice.roleSlotId,
          jobStatus: provisioningJob.status,
        });
      }
      if (choice.id === "soft_delete_generated_agents") {
        trace.event("template_generated_agent_soft_delete_selected", {
          source: "recovery_choice",
          jobStatus: provisioningJob.status,
        });
      }
      if (choice.id === "enter_group") {
        if (provisioningJob.createdGroupId) {
          await refreshSnapshot();
          onCreated(provisioningJob.createdGroupId);
        }
        return;
      }
      if (choice.id === "cancel") {
        setLaunchingGroup(true);
        setLaunchError(null);
        try {
          const canceled = await cancelGroupProvisioningJob(provisioningJob.id, { choice: "cancel_only" });
          setProvisioningJob(canceled);
          window.localStorage.removeItem(activeJobStorageKey);
        } catch (err) {
          setLaunchError(err instanceof Error ? err.message : "Cancel failed.");
        } finally {
          setLaunchingGroup(false);
        }
        return;
      }
      setLaunchingGroup(true);
      setLaunchError(null);
      try {
        const retried = await retryGroupProvisioningJob(provisioningJob.id, {
          choice: choice.id,
          ...(choice.roleSlotId ? { roleSlotId: choice.roleSlotId } : {}),
        });
        setProvisioningJob(retried);
        if (!isGroupProvisioningTerminal(retried)) {
          await advanceProvisioningJob(retried.id);
        }
      } catch (err) {
        setLaunchError(err instanceof Error ? err.message : "Recovery action failed.");
      } finally {
        setLaunchingGroup(false);
      }
    },
    [activeJobStorageKey, advanceProvisioningJob, onCreated, provisioningJob, refreshSnapshot],
  );

  const handleCancelProvisioning = useCallback(
    async (cleanup: boolean) => {
      if (!provisioningJob) return;
      if (cleanup) {
        trace.event("template_generated_agent_soft_delete_selected", {
          source: "cancel_action",
          jobStatus: provisioningJob.status,
        });
      }
      setLaunchingGroup(true);
      setLaunchError(null);
      try {
        const canceled = await cancelGroupProvisioningJob(provisioningJob.id, {
          choice: cleanup ? "soft_delete_generated_agents" : "cancel_only",
        });
        setProvisioningJob(canceled);
        window.localStorage.removeItem(activeJobStorageKey);
      } catch (err) {
        setLaunchError(err instanceof Error ? err.message : "Cancel failed.");
      } finally {
        setLaunchingGroup(false);
      }
    },
    [activeJobStorageKey, provisioningJob],
  );

  const handleCreateGroup = useCallback(async () => {
    if (!groupName.trim()) {
      setCreateGroupError("Group name is required");
      requestAnimationFrame(() => groupNameRef.current?.focus());
      return;
    }
    if (groupSelectedIds.size === 0) {
      setCreateGroupError("Select at least one member");
      return;
    }
    setCreateGroupError(null);
    const span = trace.start("create_group", { name: groupName.trim(), memberCount: groupSelectedIds.size });
    try {
      const conv = await apiFetch<Conversation>("/v1/groups", sessionToken, {
        method: "POST",
        body: JSON.stringify({
          actor_id: principalId,
          name: groupName.trim(),
          description: null,
          avatar_url: null,
          member_ids: Array.from(groupSelectedIds),
          ...(activeCompanyId ? { workspace_id: activeCompanyId } : {}),
        }),
      });

      // If 2+ agents in the group, auto-enable agent-to-agent routing
      const agentMemberCount = Array.from(groupSelectedIds).filter((id) =>
        isAgent(agents, id),
      ).length;
      if (agentMemberCount >= 2) {
        try {
          await apiFetch(
            `/v1/runtime/policies/${conv.id}`,
            sessionToken,
            {
              method: "PUT",
              body: JSON.stringify({ allow_agent_to_agent: true }),
            },
          );
        } catch (err) {
          trace.event("routing_policy_error", { convId: conv.id, error: String(err) });
        }
      }

      span.end({ status: 200 });
      await refreshSnapshot();
      onCreated(conv.id);
    } catch (err) {
      span.end({ error: err instanceof Error ? err.message : String(err) });
      setCreateGroupError(
        err instanceof Error ? err.message : "Failed to create group",
      );
    }
  }, [
    groupName,
    groupSelectedIds,
    sessionToken,
    principalId,
    activeCompanyId,
    agents,
    refreshSnapshot,
    onCreated,
  ]);

  const templateMode = selectedGroupTemplate && groupTemplateDraft;
  // Switching templates resets the draft and provisioning job, so the selector
  // stays locked while a launch is polling or its job still needs attention.
  const provisioningLocked =
    launchingGroup ||
    (provisioningJob !== null &&
      (!isGroupProvisioningTerminal(provisioningJob) || provisioningJob.recoveryChoices.length > 0));

  return (
    <Modal title="Create Group" onClose={onClose} className="modal-card-lg" closeDisabled={launchingGroup}>
      <div className="modal-form">
        <label>
          Start with
          <select
            value={selectedGroupTemplateId}
            onChange={(e) => handleGroupTemplateSelect(e.target.value)}
            disabled={provisioningLocked}
          >
            <option value="">Blank Group</option>
            {groupTemplates.map((template) => (
              <option key={template.id} value={template.id}>
                {template.name}
              </option>
            ))}
          </select>
        </label>

        {!templateMode ? (
          <>
            <label>
              Group name
              <input
                ref={groupNameRef}
                value={groupName}
                onChange={(e) => {
                  setGroupName(e.target.value);
                  if (createGroupError === "Group name is required") {
                    setCreateGroupError(null);
                  }
                }}
                placeholder="e.g. Engineering Team"
                autoFocus
                required
                aria-invalid={createGroupError === "Group name is required"}
                aria-describedby={createGroupError === "Group name is required" ? "create-group-error" : undefined}
              />
            </label>

            {groupSelectedIds.size > 0 && (
              <div className="selected-chips">
                {Array.from(groupSelectedIds).map((id) => {
                  const c = groupCandidates.find((x) => x.id === id);
                  return (
                    <button
                      type="button"
                      key={id}
                      className="chip"
                      onClick={() => toggleGroupMember(id)}
                      aria-label={`Remove ${c?.name ?? id.slice(0, 8)}`}
                    >
                      {c?.name ?? id.slice(0, 8)}
                      <X size={12} className="chip-remove" aria-hidden="true" />
                    </button>
                  );
                })}
              </div>
            )}

            <input
              className="member-search"
              value={groupSearchQuery}
              onChange={(e) => setGroupSearchQuery(e.target.value)}
              placeholder="Search agents…"
            />

            <div className="member-picker">
              {filteredGroupCandidates.length === 0 ? (
                <div className="picker-empty">No agents found</div>
              ) : (
                filteredGroupCandidates.map((c) => {
                  const selected = groupSelectedIds.has(c.id);
                  return (
                    <button
                      type="button"
                      key={c.id}
                      className={`picker-item${selected ? " selected" : ""}`}
                      onClick={() => toggleGroupMember(c.id)}
                      aria-pressed={selected}
                    >
                      <span
                        className={`picker-check${selected ? " checked" : ""}`}
                      >
                        {selected ? "\u2713" : ""}
                      </span>
                      <Avatar name={c.name} size="small" />
                      <span className="picker-info">
                        <span className="picker-name">{c.name}</span>
                        <span className="picker-meta">
                          {c.type === "agent" ? "AI Agent" : "Human"}
                        </span>
                      </span>
                      <span className="picker-id">{c.id.slice(0, 8)}...</span>
                    </button>
                  );
                })
              )}
            </div>

            {createGroupError && (
              <p id="create-group-error" className="modal-form-error" role="alert" aria-live="polite">{createGroupError}</p>
            )}
            <div className="modal-actions">
              <button className="btn-cancel" onClick={onClose}>
                Cancel
              </button>
              <button className="btn-primary" onClick={handleCreateGroup}>
                Create ({groupSelectedIds.size} selected)
              </button>
            </div>
          </>
        ) : (
          <>
            <StepTabs label="Create group steps" steps={CREATE_GROUP_STEPS} active={step} />

            {step === "setup" ? (
              <>
                <div className="create-agent-template-summary">
                  <strong>{selectedGroupTemplate.name}</strong>
                  <p>{selectedGroupTemplate.description}</p>
                  <p>{selectedGroupTemplate.recommendedAccessModel}</p>
                </div>

                <label>
                  Group name
                  <input
                    value={groupTemplateDraft.groupName}
                    onChange={(e) =>
                      updateGroupDraft((draft) => ({
                        ...draft,
                        groupName: e.target.value,
                      }))
                    }
                    onInput={() => setGroupNameEdited(true)}
                  />
                </label>

                <label>
                  Mission
                  <textarea
                    value={groupTemplateDraft.mission}
                    onChange={(e) =>
                      updateGroupDraft((draft) => {
                        const updated = updateDraftMission(draft, e.target.value);
                        return groupNameEdited ? { ...updated, groupName: draft.groupName } : updated;
                      })
                    }
                    placeholder="What should this group accomplish?"
                  />
                </label>

                <label>
                  Group default driver
                  <DriverSelect
                    value={groupTemplateDraft.groupDefaultDriver}
                    onChange={(driver) =>
                      updateGroupDraft((draft) => applyGroupDefaultDriver(draft, driver))
                    }
                    drivers={GROUP_TEMPLATE_DRIVER_IDS}
                  />
                </label>

                <div className="create-agent-template-summary">
                  <strong>Workflow</strong>
                  <p>{selectedGroupTemplate.workflow.description}</p>
                  <div className="create-agent-chip-row">
                    {selectedGroupTemplate.workflow.steps.map((item) => (
                      <span key={item}>{item}</span>
                    ))}
                  </div>
                </div>

                <div className="group-template-role-list">
                  {groupTemplateDraft.roleDrafts.map((roleDraft) => {
                    const slot = selectedGroupTemplate.roleSlots.find((slotItem) => slotItem.id === roleDraft.slotId);
                    const roleTemplate = getRoleTemplate(roleDraft.roleTemplateId);
                    const roleIssues = templateIssues.filter((issue) => issue.roleSlotId === roleDraft.slotId);
                    return (
                      <section key={roleDraft.slotId} className="group-template-role-row">
                        <div className="group-template-role-header">
                          <div>
                            <strong>{slot?.label ?? roleDraft.slotId}</strong>
                            <span>{roleTemplate?.name ?? roleDraft.roleTemplateId} · {roleDraft.required ? "Required" : "Optional"}</span>
                          </div>
                          <select
                            value={roleDraft.action}
                            onChange={(e) => {
                              const action = e.target.value as GroupTemplateRoleDraft["action"];
                              if (action === "create") {
                                resetRoleDraft(roleDraft.slotId);
                              } else {
                                updateRoleDraft(roleDraft.slotId, { action });
                              }
                            }}
                          >
                            <option value="create">Create new</option>
                            <option value="reuse">Reuse existing</option>
                            {!roleDraft.required ? <option value="skip">Skip</option> : null}
                          </select>
                        </div>

                        {roleDraft.action === "create" ? (
                          <div className="group-template-role-grid">
                            <label>
                              Agent name
                              <input
                                value={roleDraft.agentName}
                                onChange={(e) => updateRoleDraft(roleDraft.slotId, { agentName: e.target.value })}
                              />
                            </label>
                            <label>
                              Driver
                              <DriverSelect
                                value={roleDraft.driver}
                                onChange={(driver) => updateRoleDraft(roleDraft.slotId, {
                                  driver,
                                  harnessAccountId: "",
                                  harnessAccountName: "",
                                  harnessAccountModels: [],
                                  model: "",
                                })}
                                drivers={GROUP_TEMPLATE_DRIVER_IDS}
                              />
                            </label>
                            {multiHarnessAccounts ? (
                              <HarnessAccountPicker
                                companyId={activeCompanyId}
                                runtimeHostId=""
                                driver={roleDraft.driver}
                                value={roleDraft.harnessAccountId}
                                onChange={(account) => updateRoleDraft(roleDraft.slotId, {
                                  harnessAccountId: account?.id ?? "",
                                  harnessAccountName: account?.name ?? "",
                                  harnessAccountModels: account?.models ?? [],
                                  model: "",
                                })}
                              />
                            ) : null}
                            <DriverModelPicker
                              driver={roleDraft.driver}
                              model={roleDraft.model}
                              onChange={(model) => updateRoleDraft(roleDraft.slotId, { model })}
                              accountModels={roleDraft.harnessAccountId ? roleDraft.harnessAccountModels : undefined}
                              label={`${slot?.label ?? roleDraft.slotId} Model`}
                            />
                            <label>
                              Workspace
                              <select
                                value={roleDraft.workspaceMode}
                                onChange={(e) => updateRoleDraft(roleDraft.slotId, { workspaceMode: e.target.value as GroupTemplateWorkspaceMode })}
                              >
                                <option value="generated">Generated workspace</option>
                                <option value="custom">Custom path</option>
                                <option value="none">No workspace</option>
                              </select>
                            </label>
                            {roleDraft.workspaceMode === "custom" ? (
                              <label>
                                Workspace path
                                <PathPicker
                                  value={roleDraft.workspacePath}
                                  onChange={(value) => updateRoleDraft(roleDraft.slotId, { workspacePath: value })}
                                  placeholder="/path/to/workspace"
                                  autoHome={false}
                                />
                              </label>
                            ) : null}
                            {roleTemplate?.setupInputs.map((setupInput) => (
                              <SetupInputField
                                key={setupInput.id}
                                input={setupInput}
                                value={roleDraft.setupInputs[setupInput.id] ?? ""}
                                onChange={(value) =>
                                  updateRoleDraft(roleDraft.slotId, {
                                    setupInputs: { ...roleDraft.setupInputs, [setupInput.id]: value },
                                  })
                                }
                              />
                            ))}
                            {agentSkillsEnabled && <label>
                              Skill paths
                              <textarea
                                value={roleDraft.selectedSkills.join("\n")}
                                onChange={(e) =>
                                  updateRoleDraft(roleDraft.slotId, {
                                    selectedSkills: e.target.value.split(/\n|,/).map((item) => item.trim()).filter(Boolean),
                                  })
                                }
                                placeholder={roleTemplate?.suggestedSkills.join(", ") || "Optional skill paths"}
                              />
                            </label>}
                            {roleDraft.driver === "webhook_agent" ? (
                              <>
                                <label>
                                  Webhook URL
                                  <input
                                    value={roleDraft.webhookUrl}
                                    onChange={(e) => updateRoleDraft(roleDraft.slotId, { webhookUrl: e.target.value })}
                                    placeholder="https://example.com/choruz"
                                  />
                                </label>
                                <label>
                                  Webhook secret
                                  <input
                                    value={roleDraft.webhookSecret}
                                    onChange={(e) => updateRoleDraft(roleDraft.slotId, { webhookSecret: e.target.value })}
                                    placeholder="Generated if blank"
                                  />
                                </label>
                              </>
                            ) : null}
                          </div>
                        ) : null}

                        {roleDraft.action === "reuse" ? (
                          <label>
                            Existing agent
                            <select
                              value={roleDraft.existingAgentId}
                              onChange={(e) => updateRoleDraft(roleDraft.slotId, { existingAgentId: e.target.value })}
                            >
                              <option value="">Choose an agent</option>
                              {groupCandidates.map((candidate) => (
                                <option key={candidate.id} value={candidate.id}>
                                  {candidate.name}
                                </option>
                              ))}
                            </select>
                          </label>
                        ) : null}

                        {roleIssues.length > 0 ? (
                          <div className="group-template-role-issues">
                            {roleIssues.map((issue) => (
                              <p key={`${issue.code}-${issue.roleSlotId}-${issue.message}`} className={issue.severity === "error" ? "modal-form-error" : "create-agent-warning"}>
                                {issue.message}
                              </p>
                            ))}
                          </div>
                        ) : null}
                      </section>
                    );
                  })}
                </div>

                {launchError ? <p className="modal-form-error">{launchError}</p> : null}
                {availabilityBlockingMessage ? <p className="modal-form-error">{availabilityBlockingMessage}</p> : null}
                {templateIssues.filter((issue) => !issue.roleSlotId).map((issue) => (
                  <p key={`${issue.code}-${issue.message}`} className={issue.severity === "error" ? "modal-form-error" : "create-agent-warning"}>
                    {issue.message}
                  </p>
                ))}

                <div className="modal-actions">
                  <button className="btn-cancel" onClick={onClose}>
                    Cancel
                  </button>
                  <button
                    className="btn-primary"
                    onClick={() => {
                      trace.event("template_review_viewed", {
                        flow: "group",
                        templateId: groupTemplateDraft.groupTemplateId,
                      });
                      setStep("review");
                    }}
                    disabled={blockingTemplateIssues.length > 0 || !driverAvailabilityLoaded}
                  >
                    Review & Launch
                  </button>
                </div>
              </>
            ) : null}

            {step === "review" ? (
              <>
                <div className="create-agent-review">
                  {reviewItems.map((item) => (
                    <div key={item.label}>
                      <span>{item.label}</span>
                      <strong>{item.value}</strong>
                    </div>
                  ))}
                </div>
                <div className="create-agent-template-summary">
                  <strong>Kickoff</strong>
                  <p>{groupTemplateDraft.kickoffText}</p>
                </div>
                <div className="group-template-review-table">
                  {groupTemplateDraft.roleDrafts.map((role) => (
                    <div key={role.slotId}>
                      <span>{role.slotId}</span>
                      <strong>
                        {role.action === "create"
                          ? `${role.agentName} · ${driverDisplayName(role.driver)} · ${role.model || "Harness default"}`
                          : role.action === "reuse"
                            ? `Reuse ${role.existingAgentId || "agent"}`
                            : "Skipped"}
                      </strong>
                    </div>
                  ))}
                </div>
                {launchError ? <p className="modal-form-error">{launchError}</p> : null}
                {availabilityBlockingMessage ? <p className="modal-form-error">{availabilityBlockingMessage}</p> : null}
                <div className="modal-actions">
                  <button className="btn-cancel" onClick={() => setStep("setup")} disabled={launchingGroup}>
                    Back
                  </button>
                  <button className="btn-primary" onClick={handleLaunchTemplateGroup} disabled={launchingGroup || !driverAvailabilityLoaded}>
                    {launchingGroup ? "Launching…" : "Launch Group"}
                  </button>
                </div>
              </>
            ) : null}

            {step === "progress" ? (
              <>
                {provisioningJob ? (
                  <div className="group-provisioning-progress">
                    <div className="create-agent-template-summary">
                      <strong>{provisioningJob.status.replaceAll("_", " ")}</strong>
                      {provisioningJob.errorSummary ? <p>{provisioningJob.errorSummary}</p> : null}
                    </div>
                    {provisioningJob.progressSteps.map((progressStep) => (
                      <div key={progressStep.id} className={`group-provisioning-step ${progressStep.status}`}>
                        <span>{progressStep.label}</span>
                        <strong>{progressStep.status}</strong>
                      </div>
                    ))}
                    <GroupProvisioningIssueList issues={provisioningJob.issues} />
                    {launchError ? <p className="modal-form-error">{launchError}</p> : null}
                    {provisioningJob.recoveryChoices.length > 0 ? (
                      <div className="group-recovery-actions">
                        {provisioningJob.recoveryChoices.map((choice) => (
                          <button
                            key={`${choice.id}-${choice.roleSlotId ?? ""}`}
                            className={choice.destructive ? "btn-cancel" : "btn-secondary"}
                            onClick={() => handleRecoveryChoice(choice)}
                            disabled={launchingGroup}
                          >
                            {choice.label}
                          </button>
                        ))}
                      </div>
                    ) : null}
                    <div className="modal-actions">
                      {provisioningJob.createdGroupId ? (
                        <button
                          className="btn-primary"
                          onClick={async () => {
                            await refreshSnapshot();
                            onCreated(provisioningJob.createdGroupId!);
                          }}
                        >
                          Open Group
                        </button>
                      ) : null}
                      {!["completed", "completed_with_warning", "rolled_back", "canceled"].includes(provisioningJob.status) ? (
                        <>
                          {!isGroupProvisioningTerminal(provisioningJob) ? (
                            <button
                              className="btn-secondary"
                              onClick={() => advanceProvisioningJob(provisioningJob.id)}
                              disabled={launchingGroup}
                            >
                              Resume
                            </button>
                          ) : null}
                          <button className="btn-cancel" onClick={() => handleCancelProvisioning(false)} disabled={launchingGroup}>
                            Cancel
                          </button>
                          <button className="btn-cancel" onClick={() => handleCancelProvisioning(true)} disabled={launchingGroup}>
                            Soft-delete Generated
                          </button>
                        </>
                      ) : null}
                    </div>
                  </div>
                ) : (
                  <div className="picker-empty">No launch job found</div>
                )}
              </>
            ) : null}
          </>
        )}
      </div>
    </Modal>
  );
}
