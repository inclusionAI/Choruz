"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { trace } from "../../lib/api/choruz-trace";
import { Modal } from "../ui/modal";
import type { Principal, Conversation } from "../../lib/api/choruz-types";
import { PathPicker } from "../workspace/path-picker";
import { FolderPickerModal } from "../workspace/folder-picker-modal";
import { AgentInstructionForm } from "./agent-instruction-form";
import { emptyFields, type AgentInstructionFields } from "../../lib/agents/agent-instructions";
import {
  buildCreateAgentProvisioningPayload,
  buildCreateAgentReviewItems,
  createTemplateDraft,
  driverWarnings,
  groupedRoleTemplates,
  instructionStatusLabel,
  regenerateTemplateInstructions,
  templateBlockingIssues,
  type CreateAgentWorkspaceMode,
} from "../../lib/agents/create-agent-template-flow";
import { creatableAgentDriverIds, driverDisplayName } from "../../lib/drivers/driver-registry";
import { useDriverAvailability } from "../../hooks/use-driver-availability";
import { DriverSelect } from "./driver-select";
import { SetupInputField } from "../groups/setup-input-field";
import { StepTabs } from "../groups/step-tabs";
import {
  getRoleTemplate,
  type DriverId,
  type InstructionStatus,
  type SetupInputValues,
} from "../../lib/groups/team-templates";
import { DriverModelPicker } from "./driver-model-picker";
import { listRuntimeHosts } from "../../lib/api/choruz-api";
import type { RuntimeHost } from "../../lib/remote/remote-control";
import type { HarnessAccount } from "../../lib/agents/harness-accounts";
import { HarnessAccountPicker } from "./harness-account-picker";
import { transportFetch } from "../../lib/api/transport";

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export type CreateAgentModalProps = {
  activeCompanyId?: string | null;
  sessionToken: string;
  onClose: () => void;
  onCreated: (conversationId: string) => void;
  refreshSnapshot: () => Promise<void>;
  agentSkillsEnabled: boolean;
  mathcodeEnabled: boolean;
  /** The company's switch: when off, the Agent uses the login its device already has. */
  multiHarnessAccounts: boolean;
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

const CREATE_AGENT_STEPS = [
  { id: "form", label: "Setup" },
  { id: "review", label: "Review & Create" },
] as const;

export function CreateAgentModal({
  activeCompanyId,
  sessionToken,
  onClose,
  onCreated,
  refreshSnapshot,
  agentSkillsEnabled,
  mathcodeEnabled,
  multiHarnessAccounts,
}: CreateAgentModalProps) {
  const [step, setStep] = useState<"form" | "review">("form");
  const [selectedRoleTemplateId, setSelectedRoleTemplateId] = useState("");
  const [setupInputValues, setSetupInputValues] = useState<SetupInputValues>({});
  const [instructionStatus, setInstructionStatus] =
    useState<InstructionStatus>("template_default");
  const [newAgentName, setNewAgentName] = useState("");
  const [newAgentDriver, setNewAgentDriver] = useState<DriverId>("claude_terminal");
  const [newAgentModel, setNewAgentModel] = useState("");
  const [webhookUrl, setWebhookUrl] = useState("");
  const [webhookSecretInput, setWebhookSecretInput] = useState("");
  const [installResult, setInstallResult] = useState<{
    agentName: string;
    agentId: string;
    agentSecret: string;
    webhookSecret?: string;
  } | null>(null);
  const [instructionFields, setInstructionFields] = useState<AgentInstructionFields>(emptyFields());
  const [newAgentUseCustomPath, setNewAgentUseCustomPath] = useState(false);
  const [newAgentWorkspacePath, setNewAgentWorkspacePath] = useState("");
  const [showWorkspacePicker, setShowWorkspacePicker] = useState(false);
  const [showInstructions, setShowInstructions] = useState(false);
  const [createAgentError, setCreateAgentError] = useState<string | null>(null);
  const [provisioningAgent, setProvisioningAgent] = useState(false);
  const { availability: driverAvailability } = useDriverAvailability();
  const [runtimeHosts, setRuntimeHosts] = useState<RuntimeHost[]>([]);
  const [runtimeHostId, setRuntimeHostId] = useState("");
  const [harnessAccount, setHarnessAccount] = useState<HarnessAccount | null>(null);
  const runtimeHostsCompanyIdRef = useRef(activeCompanyId);

  // Skills loading
  const [skillsDir, setSkillsDir] = useState("");
  const [availableSkills, setAvailableSkills] = useState<
    { name: string; dir: string; skill_md_path: string }[]
  >([]);
  const [selectedSkillPaths, setSelectedSkillPaths] = useState<Set<string>>(
    new Set(),
  );
  const [scanningSkills, setScanningSkills] = useState(false);
  const selectedRoleTemplate = useMemo(
    () => (selectedRoleTemplateId ? getRoleTemplate(selectedRoleTemplateId) ?? null : null),
    [selectedRoleTemplateId],
  );
  const roleTemplateGroups = useMemo(() => groupedRoleTemplates(), []);
  const creatableDrivers = useMemo(() => creatableAgentDriverIds(mathcodeEnabled), [mathcodeEnabled]);
  const workspaceMode: CreateAgentWorkspaceMode = newAgentUseCustomPath
    ? "custom"
    : "generated";
  const currentDriverWarnings = useMemo(
    () =>
      driverWarnings({
        roleTemplate: selectedRoleTemplate,
        driver: newAgentDriver,
        availability: driverAvailability,
      }),
    [selectedRoleTemplate, newAgentDriver, driverAvailability],
  );
  const blockingTemplateIssues = useMemo(
    () =>
      templateBlockingIssues({
        roleTemplate: selectedRoleTemplate,
        driver: newAgentDriver,
        availability: driverAvailability,
        setupInputs: setupInputValues,
        webhookUrl,
        webhookSecret: webhookSecretInput,
      }),
    [
      selectedRoleTemplate,
      newAgentDriver,
      driverAvailability,
      setupInputValues,
      webhookUrl,
      webhookSecretInput,
    ],
  );
  const selectedSkillNames = useMemo(
    () =>
      availableSkills
        .filter((skill) => selectedSkillPaths.has(skill.dir))
        .map((skill) => skill.name),
    [availableSkills, selectedSkillPaths],
  );
  const reviewItems = useMemo(
    () =>
      buildCreateAgentReviewItems({
        agentName: newAgentName,
        driver: newAgentDriver,
        model: newAgentModel,
        runtimeHostName: runtimeHosts.find((host) => host.id === runtimeHostId)?.name,
        harnessAccountName: harnessAccount?.name,
        workspaceMode,
        workspacePath: newAgentWorkspacePath,
        roleTemplate: selectedRoleTemplate,
        selectedSkillNames,
        instructionStatus,
        webhookUrl,
        webhookSecretProvided: Boolean(webhookSecretInput.trim()),
      }),
    [
      instructionStatus,
      newAgentDriver,
      newAgentModel,
      newAgentName,
      harnessAccount,
      runtimeHostId,
      runtimeHosts,
      newAgentWorkspacePath,
      selectedRoleTemplate,
      selectedSkillNames,
      webhookSecretInput,
      webhookUrl,
      workspaceMode,
    ],
  );

  useEffect(() => {
    let cancelled = false;
    const companyChanged = runtimeHostsCompanyIdRef.current !== activeCompanyId;
    runtimeHostsCompanyIdRef.current = activeCompanyId;
    if (companyChanged) {
      setRuntimeHostId("");
      setHarnessAccount(null);
    }
    if (!activeCompanyId) {
      setRuntimeHosts([]);
      return;
    }
    void listRuntimeHosts(sessionToken, activeCompanyId)
      .then((hosts) => {
        if (!cancelled) {
          const availableHosts = hosts.filter((host) => host.status !== "revoked");
          setRuntimeHosts(availableHosts);
          setRuntimeHostId((current) =>
            availableHosts.some((host) => host.id === current) ? current : "",
          );
        }
      })
      .catch(() => {
        if (!cancelled) {
          setRuntimeHosts([]);
          setRuntimeHostId("");
        }
      });
    return () => { cancelled = true; };
  }, [activeCompanyId, sessionToken]);

  const confirmTemplateOverwrite = useCallback(() => {
    if (!selectedRoleTemplate || instructionStatus !== "customized") return true;
    return window.confirm(
      "Replace your customized instructions with the current template defaults?",
    );
  }, [selectedRoleTemplate, instructionStatus]);

  const applyTemplateInstructions = useCallback(
    (template = selectedRoleTemplate, values = setupInputValues) => {
      if (!template) return;
      const rendered = regenerateTemplateInstructions({
        roleTemplate: template,
        setupInputs: values,
        workspaceMode,
        workspacePath: newAgentWorkspacePath,
      });
      setInstructionFields(rendered.instructionFields);
      setInstructionStatus(rendered.instructionStatus);
    },
    [newAgentWorkspacePath, selectedRoleTemplate, setupInputValues, workspaceMode],
  );

  const handleTemplateSelect = useCallback(
    (templateId: string) => {
      setCreateAgentError(null);
      setStep("form");
      if (templateId === "") {
        if (!confirmTemplateOverwrite()) return;
        setSelectedRoleTemplateId("");
        setSetupInputValues({});
        setInstructionFields(emptyFields());
        setInstructionStatus("template_default");
        setNewAgentName("");
        return;
      }
      const template = getRoleTemplate(templateId);
      if (!template) return;
      if (!confirmTemplateOverwrite()) return;
      trace.event("template_template_selected", {
        flow: "agent",
        templateId: template.id,
        templateVersion: template.version,
      });
      const draft = createTemplateDraft({
        roleTemplate: template,
        workspaceMode,
        workspacePath: newAgentWorkspacePath,
      });
      setSelectedRoleTemplateId(template.id);
      setSetupInputValues(draft.setupInputs);
      setNewAgentName(draft.agentName);
      setNewAgentDriver(draft.driver);
      setNewAgentModel("");
      setInstructionFields(draft.instructionFields);
      setInstructionStatus(draft.instructionStatus);
      setShowInstructions(true);
    },
    [confirmTemplateOverwrite, newAgentWorkspacePath, workspaceMode],
  );

  const handleSetupInputChange = useCallback(
    (inputId: string, value: string) => {
      if (!selectedRoleTemplate) return;
      if (!confirmTemplateOverwrite()) return;
      const nextValues = { ...setupInputValues, [inputId]: value };
      setSetupInputValues(nextValues);
      applyTemplateInstructions(selectedRoleTemplate, nextValues);
      setStep("form");
    },
    [
      applyTemplateInstructions,
      confirmTemplateOverwrite,
      selectedRoleTemplate,
      setupInputValues,
    ],
  );

  const handleInstructionChange = useCallback(
    (fields: AgentInstructionFields) => {
      setInstructionFields(fields);
      if (selectedRoleTemplate) setInstructionStatus("customized");
      setStep("form");
    },
    [selectedRoleTemplate],
  );

  const handleResetInstructions = useCallback(() => {
    applyTemplateInstructions();
    setShowInstructions(true);
    setStep("form");
  }, [applyTemplateInstructions]);

  const handleWorkspaceModeChange = useCallback(
    (checked: boolean) => {
      if (selectedRoleTemplate && instructionStatus === "customized" && !confirmTemplateOverwrite()) {
        return;
      }
      setNewAgentUseCustomPath(checked);
      if (!checked) setNewAgentWorkspacePath("");
      if (selectedRoleTemplate) {
        const rendered = regenerateTemplateInstructions({
          roleTemplate: selectedRoleTemplate,
          setupInputs: setupInputValues,
          workspaceMode: checked ? "custom" : "generated",
          workspacePath: checked ? newAgentWorkspacePath : "",
        });
        setInstructionFields(rendered.instructionFields);
        setInstructionStatus(rendered.instructionStatus);
      }
      setStep("form");
    },
    [
      confirmTemplateOverwrite,
      instructionStatus,
      newAgentWorkspacePath,
      selectedRoleTemplate,
      setupInputValues,
    ],
  );

  const handleWorkspacePathChange = useCallback(
    (path: string) => {
      if (selectedRoleTemplate && instructionStatus === "customized" && !confirmTemplateOverwrite()) {
        return;
      }
      setNewAgentWorkspacePath(path);
      if (selectedRoleTemplate) {
        const rendered = regenerateTemplateInstructions({
          roleTemplate: selectedRoleTemplate,
          setupInputs: setupInputValues,
          workspaceMode: "custom",
          workspacePath: path,
        });
        setInstructionFields(rendered.instructionFields);
        setInstructionStatus(rendered.instructionStatus);
      }
      setStep("form");
    },
    [confirmTemplateOverwrite, instructionStatus, selectedRoleTemplate, setupInputValues],
  );

  const handleDriverChange = useCallback((driver: DriverId) => {
    setNewAgentDriver(driver);
    setNewAgentModel("");
    setHarnessAccount(null);
    setStep("form");
  }, []);

  const handleScanSkills = useCallback(async () => {
    const dir = skillsDir.endsWith("/") ? skillsDir.slice(0, -1) : skillsDir;
    setScanningSkills(true);
    try {
      const res = await transportFetch("/api/skills/scan", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ path: dir }),
      });
      const data = (await res.json()) as {
        skills?: { name: string; dir: string; skill_md_path: string }[];
        error?: string;
      };
      if (!res.ok) throw new Error(data.error || "Scan failed");
      const skills = data.skills || [];
      setAvailableSkills(skills);
      setSelectedSkillPaths(new Set(skills.map((s) => s.dir)));
    } catch (err) {
      trace.event("skill_scan_error", { skillsDir, error: String(err) });
      setAvailableSkills([]);
      setSelectedSkillPaths(new Set());
    } finally {
      setScanningSkills(false);
    }
  }, [skillsDir]);

  const handleCreateAgent = useCallback(async () => {
    if (!newAgentName.trim()) {
      setCreateAgentError("Agent name is required");
      return;
    }
    if (blockingTemplateIssues.length > 0) {
      setCreateAgentError(blockingTemplateIssues.map((issue) => issue.message).join(" "));
      setStep("form");
      return;
    }
    const isWebhookMode = newAgentDriver === "webhook_agent";
    setCreateAgentError(null);
    setProvisioningAgent(true);
    const span = trace.start("create_agent", { name: newAgentName.trim(), driver: newAgentDriver });
    try {
      const payload = buildCreateAgentProvisioningPayload({
        name: newAgentName,
        driver: newAgentDriver,
        model: newAgentModel,
        instructionFields,
        activeCompanyId,
        runtimeHostId,
        harnessAccountId: harnessAccount?.id,
        workspaceMode,
        workspacePath: newAgentWorkspacePath,
        selectedSkillPaths: agentSkillsEnabled ? [...selectedSkillPaths] : [],
        webhookUrl,
        webhookSecret: webhookSecretInput,
        roleTemplate: selectedRoleTemplate,
        setupInputs: setupInputValues,
        instructionStatus,
      });
      const provisionRes = await transportFetch("/api/agents/provision", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(payload),
      });
      if (!provisionRes.ok) {
        const err = (await provisionRes
          .json()
          .catch(() => ({ error: provisionRes.statusText }))) as {
          error?: string;
        };
        throw new Error(
          err.error ?? `Provision failed: ${provisionRes.status}`,
        );
      }
      const result = (await provisionRes.json()) as {
        agent: Principal;
        secret: string;
        conversation: Conversation;
        binding: { id: string };
        workspace_path: string;
        webhook_secret?: string;
      };
      span.end({ status: 200 });
      if (selectedRoleTemplate) {
        trace.event("template_agent_created_from_template", {
          roleTemplateId: selectedRoleTemplate.id,
          roleTemplateVersion: selectedRoleTemplate.version,
          driver: newAgentDriver,
          workspaceMode,
          instructionStatus,
        });
      }
      await refreshSnapshot();
      if (isWebhookMode) {
        // Show the secret once — Slack-style install confirmation. The
        // user must copy it now; rotate afterwards if lost.
        setInstallResult({
          agentName: result.agent.name,
          agentId: result.agent.id,
          agentSecret: result.secret,
          webhookSecret: result.webhook_secret,
        });
      } else {
        onCreated(result.conversation.id);
      }
    } catch (err) {
      span.end({ error: err instanceof Error ? err.message : String(err) });
      setCreateAgentError(
        err instanceof Error ? err.message : "Failed to create agent",
      );
    } finally {
      setProvisioningAgent(false);
    }
  }, [
    newAgentName,
    newAgentDriver,
    newAgentModel,
    webhookUrl,
    webhookSecretInput,
    instructionFields,
    activeCompanyId,
    runtimeHostId,
    harnessAccount,
    sessionToken,
    workspaceMode,
    newAgentWorkspacePath,
    agentSkillsEnabled,
    selectedSkillPaths,
    selectedRoleTemplate,
    setupInputValues,
    instructionStatus,
    blockingTemplateIssues,
    refreshSnapshot,
    onCreated,
  ]);

  if (installResult) {
    return (
      <Modal title="External agent installed" onClose={onClose}>
        <div className="modal-form">
          <p>
            <strong>{installResult.agentName}</strong> is now a member of
            your workspace. The two secrets below are shown{" "}
            <strong>once</strong> — copy them into your app now.
          </p>
          <label>
            Agent bearer secret (for <code>Authorization: Bearer</code>)
            <input readOnly value={installResult.agentSecret} onFocus={(e) => e.currentTarget.select()} />
          </label>
          {installResult.webhookSecret ? (
            <label>
              Webhook signing secret (for <code>X-Choruz-Signature</code> verification)
              <input readOnly value={installResult.webhookSecret} onFocus={(e) => e.currentTarget.select()} />
            </label>
          ) : null}
          <label>
            Agent ID
            <input readOnly value={installResult.agentId} onFocus={(e) => e.currentTarget.select()} />
          </label>
          <div className="modal-actions">
            <button
              className="btn-primary"
              onClick={() => {
                setInstallResult(null);
                onClose();
              }}
            >
              Done
            </button>
          </div>
        </div>
      </Modal>
    );
  }

  return (
    <>
      <Modal title="Create Agent" onClose={onClose} closeDisabled={provisioningAgent}>
        <div className="modal-form">
          <StepTabs label="Create agent steps" steps={CREATE_AGENT_STEPS} active={step} />
          {step === "form" ? (
            <>
          <label>
            Start with
            <select
              value={selectedRoleTemplateId}
              onChange={(e) => handleTemplateSelect(e.target.value)}
            >
              <option value="">Blank Agent</option>
              {roleTemplateGroups.map((group) => (
                <optgroup key={group.category} label={group.label}>
                  {group.templates.map((template) => (
                    <option key={template.id} value={template.id}>
                      {template.name}
                    </option>
                  ))}
                </optgroup>
              ))}
            </select>
          </label>
          {selectedRoleTemplate ? (
            <div className="create-agent-template-summary">
              <strong>{selectedRoleTemplate.name}</strong>
              <p>{selectedRoleTemplate.description}</p>
              <div className="create-agent-chip-row">
                <span>Recommended: {driverDisplayName(selectedRoleTemplate.recommendedDriver)}</span>
                <span>
                  Compatible:{" "}
                  {[
                    ...new Set(
                      selectedRoleTemplate.compatibleDrivers.map(driverDisplayName),
                    ),
                  ].join(", ")}
                </span>
              </div>
            </div>
          ) : null}
          <label>
            Agent name
            <input
              value={newAgentName}
              onChange={(e) => setNewAgentName(e.target.value)}
              placeholder="e.g. Coder, Reviewer, Architect"
              autoFocus
            />
          </label>
          <label>
            Driver
            <DriverSelect
              aria-label="Driver"
              value={newAgentDriver}
              onChange={handleDriverChange}
              drivers={creatableDrivers}
            />
          </label>
          {newAgentDriver !== "webhook_agent" && runtimeHosts.length > 0 ? (
            <label>
              Run on
              <select
                aria-label="Runtime server"
                value={runtimeHostId}
                onChange={(event) => {
                  setRuntimeHostId(event.target.value);
                  setHarnessAccount(null);
                  setNewAgentModel("");
                }}
              >
                <option value="">This computer</option>
                {runtimeHosts.map((host) => (
                  <option key={host.id} value={host.id} disabled={host.status !== "online"}>
                    {host.name}{host.status === "online" ? "" : " (offline)"}
                  </option>
                ))}
              </select>
              <span className="field-hint">The Agent keeps this placement until you move it.</span>
            </label>
          ) : null}
          {multiHarnessAccounts ? (
            <HarnessAccountPicker
              companyId={activeCompanyId}
              runtimeHostId={runtimeHostId}
              driver={newAgentDriver}
              value={harnessAccount?.id ?? ""}
              onChange={(account) => {
                setHarnessAccount(account);
                setNewAgentModel("");
                setStep("form");
              }}
            />
          ) : null}
          {currentDriverWarnings.map((warning) => (
            <p key={warning.code} className="create-agent-warning">
              {warning.message}
            </p>
          ))}
          {newAgentDriver !== "mathcode_terminal" && (
            <DriverModelPicker
              driver={newAgentDriver}
              model={newAgentModel}
              onChange={(model) => {
                setNewAgentModel(model);
                setStep("form");
              }}
              accountModels={harnessAccount ? harnessAccount.models : undefined}
            />
          )}
          {newAgentDriver === "mathcode_terminal" && (
            <p className="field-hint">MathCode chooses its model through its own local configuration.</p>
          )}
          {newAgentDriver === "webhook_agent" && (
            <>
              <label>
                Webhook URL
                <input
                  type="url"
                  value={webhookUrl}
                  onChange={(e) => setWebhookUrl(e.target.value)}
                  placeholder="https://hermes.example.com/choruz/hook"
                />
              </label>
              <label>
                Signing secret (optional — leave empty to auto-generate)
                <input
                  type="text"
                  value={webhookSecretInput}
                  onChange={(e) => setWebhookSecretInput(e.target.value)}
                  placeholder="auto-generated if empty"
                />
              </label>
              <p className="field-hint">
                Choruz will POST <code>message.created</code> events to this URL.
                The signing secret verifies the <code>X-Choruz-Signature</code>{" "}
                header (<code>sha256=HMAC(secret, body)</code>). The agent uses
                its bearer secret to post replies back via{" "}
                <code>POST /v1/messages</code>.
              </p>
            </>
          )}
          {selectedRoleTemplate && selectedRoleTemplate.setupInputs.length > 0 ? (
            <div className="create-agent-template-inputs">
              <div className="create-agent-section-label">Template setup</div>
              {selectedRoleTemplate.setupInputs.map((input) => (
                <SetupInputField
                  key={input.id}
                  input={input}
                  value={setupInputValues[input.id] ?? ""}
                  onChange={(value) => handleSetupInputChange(input.id, value)}
                />
              ))}
              {agentSkillsEnabled && <div>
                <div className="create-agent-section-label">Suggested skills</div>
                <div className="create-agent-chip-row">
                  {selectedRoleTemplate.suggestedSkills.map((skill) => (
                    <span key={skill}>{skill}</span>
                  ))}
                </div>
              </div>}
              <div>
                <div className="create-agent-section-label">First-task examples</div>
                <ul className="create-agent-template-list">
                  {selectedRoleTemplate.suggestedFirstTasks.map((task) => (
                    <li key={task}>{task}</li>
                  ))}
                </ul>
              </div>
            </div>
          ) : null}
          <label className="create-agent-checkbox-label">
            <input
              type="checkbox"
              checked={newAgentUseCustomPath}
              onChange={(e) => handleWorkspaceModeChange(e.target.checked)}
            />
            Custom workspace path
          </label>
          {newAgentUseCustomPath && (
            <div className="create-agent-field-inset">
              <div className="workspace-session-folder-row">
                <PathPicker
                  value={newAgentWorkspacePath}
                  onChange={handleWorkspacePathChange}
                  placeholder="/path/to/workspace"
                />
                <button type="button" className="server-manager-btn" onClick={() => setShowWorkspacePicker(true)}>
                  Browse
                </button>
              </div>
              <p className="field-hint">
                Select an existing directory. Leave unchecked to auto-generate.
              </p>
            </div>
          )}
          {/* Skills loading */}
          {agentSkillsEnabled && <div>
            <div className="create-agent-section-label">Skills (optional)</div>
            <div className="create-agent-skills-row">
              <div>
                <PathPicker
                  value={skillsDir}
                  onChange={setSkillsDir}
                  placeholder="Select a directory with .md skill files"
                  autoHome={false}
                />
              </div>
              <button
                type="button"
                className="btn-primary"
                disabled={!skillsDir || scanningSkills}
                onClick={handleScanSkills}
              >
                {scanningSkills ? "Scanning…" : "Scan"}
              </button>
            </div>
          </div>}
          {agentSkillsEnabled && scanningSkills && (
            <p className="detail-inline-empty">Scanning…</p>
          )}
          {agentSkillsEnabled && !scanningSkills && availableSkills.length > 0 && (
            <div className="create-agent-field-inset">
              <label className="create-agent-select-all">
                <input
                  type="checkbox"
                  checked={selectedSkillPaths.size === availableSkills.length}
                  onChange={(e) => {
                    if (e.target.checked) {
                      setSelectedSkillPaths(
                        new Set(availableSkills.map((s) => s.dir)),
                      );
                    } else {
                      setSelectedSkillPaths(new Set());
                    }
                  }}
                />
                Select All ({availableSkills.length})
              </label>
              <div className="create-agent-skills-list">
                {availableSkills.map((skill) => (
                  <label key={skill.dir}>
                    <input
                      type="checkbox"
                      checked={selectedSkillPaths.has(skill.dir)}
                      onChange={(e) => {
                        setSelectedSkillPaths((prev) => {
                          const next = new Set(prev);
                          if (e.target.checked) {
                            next.add(skill.dir);
                          } else {
                            next.delete(skill.dir);
                          }
                          return next;
                        });
                      }}
                    />
                    {skill.name}
                  </label>
                ))}
              </div>
            </div>
          )}
          {agentSkillsEnabled && !scanningSkills &&
            skillsDir.endsWith("/") &&
            availableSkills.length === 0 &&
            skillsDir.length > 1 && (
              <p className="detail-inline-empty">
                No .md skill files found in this directory.
              </p>
            )}
          {/* Collapsible Instructions */}
          <button
            type="button"
            onClick={() => setShowInstructions(!showInstructions)}
            aria-expanded={showInstructions}
            className="create-agent-collapse-btn"
          >
            <span className="caret">&#9654;</span>
            Instructions
            {selectedRoleTemplate ? (
              <span className="create-agent-status-pill">
                {instructionStatusLabel(instructionStatus)}
              </span>
            ) : null}
          </button>
          {selectedRoleTemplate ? (
            <button
              type="button"
              className="create-agent-inline-action"
              onClick={handleResetInstructions}
              disabled={instructionStatus !== "customized"}
            >
              Reset to template
            </button>
          ) : null}
          {showInstructions && (
            <AgentInstructionForm
              fields={instructionFields}
              onChange={handleInstructionChange}
            />
          )}
          {createAgentError && (
            <p className="modal-form-error">
              {createAgentError}
            </p>
          )}
          <div className="modal-actions">
            <button
              className="btn-cancel"
              disabled={provisioningAgent}
              onClick={onClose}
            >
              Cancel
            </button>
            <button
              className="btn-primary"
              onClick={() => {
                if (!newAgentName.trim()) {
                  setCreateAgentError("Agent name is required");
                  return;
                }
                if (blockingTemplateIssues.length > 0) {
                  setCreateAgentError(blockingTemplateIssues.map((issue) => issue.message).join(" "));
                  return;
                }
                trace.event("template_review_viewed", {
                  flow: "agent",
                  templateId: selectedRoleTemplate?.id ?? "blank",
                });
                setCreateAgentError(null);
                setStep("review");
              }}
              disabled={provisioningAgent}
            >
              Review & Create
            </button>
          </div>
            </>
          ) : (
            <>
              <div className="create-agent-review">
                {reviewItems.map((item) => (
                  <div key={item.label}>
                    <span>{item.label}</span>
                    <strong>{item.value}</strong>
                  </div>
                ))}
              </div>
              {currentDriverWarnings.map((warning) => (
                <p key={warning.code} className="create-agent-warning">
                  {warning.message}
                </p>
              ))}
              {createAgentError && (
                <p className="modal-form-error">
                  {createAgentError}
                </p>
              )}
              <div className="modal-actions">
                <button
                  className="btn-cancel"
                  disabled={provisioningAgent}
                  onClick={() => setStep("form")}
                >
                  Back
                </button>
                <button
                  className="btn-primary"
                  onClick={handleCreateAgent}
                  disabled={provisioningAgent}
                >
                  {provisioningAgent ? "Creating…" : "Create Agent"}
                </button>
              </div>
            </>
          )}
        </div>
      </Modal>
      {showWorkspacePicker ? (
        <FolderPickerModal
          initialPath={newAgentWorkspacePath || undefined}
          onSelect={(path) => {
            handleWorkspacePathChange(path);
            setShowWorkspacePicker(false);
          }}
          onClose={() => setShowWorkspacePicker(false)}
        />
      ) : null}
    </>
  );
}
