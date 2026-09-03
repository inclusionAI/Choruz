import { execFileSync } from "node:child_process";
import crypto from "node:crypto";
import { promises as fs } from "node:fs";
import path from "node:path";

import {
  composeAgentInstructionTemplate,
  CORE_PROTOCOL_FILE,
  STANDARD_EXTENSION_FILES,
} from "./agent-instruction-template";
import { persistAgentToken as defaultPersistAgentToken } from "./agent-tokens";
import {
  createAgent,
  createDirectConversation,
  createRuntimeBinding,
  rotateAgentSecret,
  type Conversation,
  type Principal,
  type RuntimeBinding,
} from "../api/choruz-api";
import type { RoleTemplateAgentCreationMetadata } from "../groups/group-provisioning-contract";
import { postgresQueryClient } from "../groups/group-provisioning-db";
import {
  createGroupProvisioningStore,
  type AgentTemplateWorkspaceMode,
  type JsonValue,
} from "../groups/group-provisioning-store";
import type { InstructionStatus, TemplateVersion } from "../groups/team-templates";
import { validateModelId } from "../drivers/driver-model-validation";
import { defaultHarnessAccountForLaunch, getHarnessAccount, type HarnessAccount } from "./harness-accounts";

export type AgentProvisioningDriverType =
  | "claude_terminal"
  | "codex_exec"
  | "codex_app_server"
  | "codex_terminal"
  | "pi_terminal"
  | "grok_terminal"
  | "opencode_terminal"
  | "mathcode_terminal"
  | "webhook_agent";

export type ProvisionRequestBody = {
  name: string;
  driver_type: AgentProvisioningDriverType;
  idempotency_key?: string;
  model?: string;
  runtime_host_id?: string;
  harness_account_id?: string;
  instructions?: string;
  workspace_id?: string;
  workspace_path?: string;
  channel_visibility?: "visible" | "internal";
  skill_paths?: string[];
  webhook_url?: string;
  webhook_secret?: string;
  template_metadata?: RoleTemplateAgentCreationMetadata;
};

export type ProvisionResponse = {
  agent: Principal;
  secret: string;
  conversation: Conversation;
  binding: RuntimeBinding;
  workspace_path: string;
  webhook_secret?: string;
};

export type AgentProvisioningStepName =
  | "create_principal"
  | "persist_token"
  | "create_workspace"
  | "write_instructions"
  | "install_outbox_helper"
  | "copy_skills"
  | "create_direct_conversation"
  | "create_runtime_binding"
  | "register_webhook"
  | "store_role_template_provenance";

export type AgentProvisioningJobContext = {
  jobId: string;
  roleSlotId: string;
  idempotencyKey: string;
  allowWebhookRegistrationReplayWithoutOutput?: boolean;
};

export type AgentProvisioningStepRecord<T extends JsonValue = JsonValue> = {
  key: string;
  step: AgentProvisioningStepName;
  jobId: string;
  roleSlotId: string;
  idempotencyKey: string;
  output: T;
};

export type AgentProvisioningStepRecorder = {
  readStep<T extends JsonValue>(
    input: Omit<AgentProvisioningStepRecord, "output">,
  ): Promise<T | null>;
  recordStep<T extends JsonValue>(record: AgentProvisioningStepRecord<T>): Promise<void>;
};

export type RoleTemplateProvenanceInput = {
  roleTemplateId: string;
  roleTemplateVersion: TemplateVersion;
  instructionStatus: InstructionStatus;
  setupSummary?: JsonValue;
  workspaceMode: AgentTemplateWorkspaceMode;
  selectedSkills?: string[];
};

export type RoleTemplateProvenanceWriter = (input: {
  agentPrincipalId: string;
  roleTemplateId: string;
  roleTemplateVersion: TemplateVersion;
  instructionStatus: InstructionStatus;
  setupSummary: JsonValue;
  workspaceMode: AgentTemplateWorkspaceMode;
  selectedSkills: string[];
  originatingJobId: string | null;
}) => Promise<void>;

export type AgentProvisioningInput = {
  sessionToken: string;
  actorId: string;
  body: ProvisionRequestBody;
  jobContext?: AgentProvisioningJobContext;
  stepRecorder?: AgentProvisioningStepRecorder;
  roleTemplateProvenance?: RoleTemplateProvenanceInput;
  provenanceWriter?: RoleTemplateProvenanceWriter;
};

export type AgentProvisioningFailureDetail = {
  step: AgentProvisioningStepName;
  message: string;
  completedSteps: Partial<Record<AgentProvisioningStepName, JsonValue>>;
};

export class AgentProvisioningError extends Error {
  readonly detail: AgentProvisioningFailureDetail;

  constructor(detail: AgentProvisioningFailureDetail) {
    super(detail.message);
    this.name = "AgentProvisioningError";
    this.detail = detail;
  }
}

type WorkspaceResult = {
  workspacePath: string;
  gitWorktreePath: string | null;
  mode: "generated" | "custom" | "webhook";
  generatedByChoruz: boolean;
};

type CreatePrincipalOutput = {
  agent: Principal;
  secret?: string;
};

type PrincipalWithSensitiveFields = Principal & {
  secret_hash?: unknown;
};

type RuntimeBindingCheckpoint = Omit<RuntimeBinding, "external_session_id" | "external_thread_id">;

export type AgentProvisioningDeps = {
  createAgent: typeof createAgent;
  rotateAgentSecret: typeof rotateAgentSecret;
  persistAgentToken: typeof defaultPersistAgentToken;
  createDirectConversation: typeof createDirectConversation;
  createRuntimeBinding: typeof createRuntimeBinding;
  getHarnessAccount: typeof getHarnessAccount;
  defaultHarnessAccount: typeof defaultHarnessAccountForLaunch;
  fetch: typeof fetch;
};

export const AGENT_SCOPES = [
  "messages:read",
  "messages:write",
  "events:read",
] as const;

const defaultDeps: AgentProvisioningDeps = {
  createAgent,
  rotateAgentSecret,
  persistAgentToken: defaultPersistAgentToken,
  createDirectConversation,
  createRuntimeBinding,
  getHarnessAccount,
  defaultHarnessAccount: defaultHarnessAccountForLaunch,
  fetch,
};

export function validateProvisionRequestBody(
  body: ProvisionRequestBody,
  options: { allowChannelVisibility?: boolean } = {},
): string | null {
  const { name, driver_type: driverType, instructions } = body;
  if (!name || typeof name !== "string" || name.trim().length === 0) {
    return "Field `name` is required.";
  }
  if (
    body.idempotency_key !== undefined &&
    (typeof body.idempotency_key !== "string" ||
      body.idempotency_key.length < 1 ||
      body.idempotency_key.length > 128 ||
      !/^[A-Za-z0-9._:-]+$/.test(body.idempotency_key))
  ) {
    return "Field `idempotency_key` must be 1-128 characters using letters, digits, '.', '_', ':', or '-'.";
  }
  if (body.channel_visibility !== undefined && !options.allowChannelVisibility) {
    return "Field `channel_visibility` is not accepted by this route.";
  }
  if (
    body.channel_visibility !== undefined &&
    body.channel_visibility !== "visible" &&
    body.channel_visibility !== "internal"
  ) {
    return 'Field `channel_visibility` must be "visible" or "internal".';
  }
  if (
    driverType !== "claude_terminal" &&
    driverType !== "codex_exec" &&
    driverType !== "codex_app_server" &&
    driverType !== "codex_terminal" &&
    driverType !== "pi_terminal" &&
    driverType !== "grok_terminal" &&
    driverType !== "opencode_terminal" &&
    driverType !== "mathcode_terminal" &&
    driverType !== "webhook_agent"
  ) {
    return 'Field `driver_type` must be one of "claude_terminal", "codex_exec", "codex_app_server", "codex_terminal", "pi_terminal", "grok_terminal", "opencode_terminal", "mathcode_terminal", or "webhook_agent".';
  }
  if (driverType === "webhook_agent") {
    const webhookError = validateWebhookProvisioningConfig(body.webhook_url, body.webhook_secret);
    if (webhookError) return webhookError;
  } else if (
    instructions === undefined ||
    instructions === null ||
    typeof instructions !== "string"
  ) {
    return "Field `instructions` is required.";
  }
  const modelError = validateModelId(body.model);
  if (modelError) return modelError;
  if (driverType === "webhook_agent" && body.model?.trim()) {
    return "Field `model` is not accepted for webhook_agent.";
  }
  if (driverType === "mathcode_terminal" && body.model?.trim()) {
    return "Field `model` is not accepted for mathcode_terminal.";
  }
  if (
    body.runtime_host_id !== undefined &&
    (typeof body.runtime_host_id !== "string" || !body.runtime_host_id.trim())
  ) {
    return "Field `runtime_host_id` must be a non-empty string.";
  }
  if (driverType === "webhook_agent" && body.runtime_host_id) {
    return "Field `runtime_host_id` is not accepted for webhook_agent.";
  }
  if (
    body.harness_account_id !== undefined &&
    (typeof body.harness_account_id !== "string" || !/^[0-9a-f-]{36}$/i.test(body.harness_account_id))
  ) {
    return "Field `harness_account_id` must be a valid account id.";
  }
  if (driverType === "webhook_agent" && body.harness_account_id) {
    return "Field `harness_account_id` is not accepted for webhook_agent.";
  }
  return null;
}

export function validateWebhookProvisioningConfig(
  webhookUrl: string | undefined,
  webhookSecret: string | undefined,
): string | null {
  void webhookSecret;
  if (!webhookUrl?.trim()) {
    return "Field `webhook_url` must be an http(s) URL for webhook_agent.";
  }
  try {
    const url = new URL(webhookUrl.trim());
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      return "Field `webhook_url` must be an http(s) URL for webhook_agent.";
    }
  } catch {
    return "Field `webhook_url` must be a valid URL for webhook_agent.";
  }
  return null;
}

export function validateCustomWorkspacePath(customWorkspacePath?: string): {
  error: string;
  status: 400 | 500;
} | null {
  if (!customWorkspacePath?.trim()) return null;
  const home = process.env.HOME;
  if (!home) {
    return { error: "Cannot validate workspace path: HOME is not set.", status: 500 };
  }
  const resolved = path.resolve(customWorkspacePath.trim());
  if (resolved !== home && !resolved.startsWith(home + "/")) {
    return { error: `Workspace path must be under ${home}`, status: 400 };
  }
  return null;
}

export function sanitizeName(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

export function runtimeDir(): string {
  if (process.env.CHORUZ_RUNTIME_DIR) {
    return process.env.CHORUZ_RUNTIME_DIR;
  }
  return path.resolve(process.cwd(), "..", "..", ".choruz-runtime");
}

const DRIVER_FILES: Record<AgentProvisioningDriverType, { template: string; instructions: string }> = {
  claude_terminal: { template: "agent-claude-md-template.md", instructions: "CLAUDE.md" },
  codex_terminal: { template: "agent-codex-md-template.md", instructions: "AGENTS.md" },
  codex_exec: { template: "agent-codex-md-template.md", instructions: "AGENTS.md" },
  codex_app_server: { template: "agent-codex-md-template.md", instructions: "AGENTS.md" },
  pi_terminal: { template: "agent-codex-md-template.md", instructions: "AGENTS.md" },
  grok_terminal: { template: "agent-codex-md-template.md", instructions: "AGENTS.md" },
  opencode_terminal: { template: "agent-codex-md-template.md", instructions: "AGENTS.md" },
  mathcode_terminal: { template: "agent-codex-md-template.md", instructions: "AGENTS.md" },
  webhook_agent: { template: "agent-claude-md-template.md", instructions: "CLAUDE.md" },
};

export function templateFileForDriver(driverType: string): string {
  const entry = (DRIVER_FILES as Record<string, { template: string }>)[driverType];
  return entry?.template ?? DRIVER_FILES.claude_terminal.template;
}

export async function buildInstructionsFromTemplate(
  agentName: string,
  userInstructions: string,
  driverType: string = "claude_terminal",
): Promise<string> {
  const templateFile = templateFileForDriver(driverType);
  const templateRoots = [
    path.resolve(process.cwd(), "..", "..", "agent-templates"),
    path.resolve(process.cwd(), "agent-templates"),
  ];
  const requiredFiles = [templateFile, CORE_PROTOCOL_FILE, ...STANDARD_EXTENSION_FILES];
  let assembledParts: string[] | null = null;
  for (const root of templateRoots) {
    const reads = await Promise.allSettled(
      requiredFiles.map((file) => fs.readFile(path.join(root, file), "utf-8")),
    );
    const nonMissingFailure = reads.find((result) => {
      if (result.status === "fulfilled") return false;
      const code = (result.reason as NodeJS.ErrnoException).code;
      return code !== "ENOENT" && code !== "ENOTDIR";
    });
    if (nonMissingFailure?.status === "rejected") throw nonMissingFailure.reason;
    if (reads.some((result) => result.status === "rejected")) continue;
    assembledParts = reads.map((result) => {
      if (result.status === "rejected") throw result.reason;
      return result.value;
    });
    break;
  }
  if (!assembledParts) {
    throw new Error(
      `Complete agent instructions template set for ${templateFile} not found in any candidate root: ${templateRoots.join(", ")}`,
    );
  }
  const [template, coreProtocol, ...standardExtensions] = assembledParts;

  const agentSection = userInstructions.trim()
    ? userInstructions.trim()
    : `You are ${agentName}, an AI assistant on the Choruz platform.`;
  return composeAgentInstructionTemplate(template, agentSection, {
    coreProtocol,
    standardExtensions,
  });
}

export async function copyDirRecursive(src: string, dest: string): Promise<void> {
  await fs.mkdir(dest, { recursive: true });
  const entries = await fs.readdir(src, { withFileTypes: true });
  for (const entry of entries) {
    const srcPath = path.join(src, entry.name);
    const destPath = path.join(dest, entry.name);
    if (entry.isDirectory()) {
      await copyDirRecursive(srcPath, destPath);
    } else {
      await fs.copyFile(srcPath, destPath);
    }
  }
}

export async function installOutboxHelper(workspacePath: string): Promise<void> {
  const choruzDir = path.join(workspacePath, ".choruz");
  const outboxDir = path.join(workspacePath, ".choruz-outbox");
  await fs.mkdir(path.join(outboxDir, "tmp"), { recursive: true });
  await fs.mkdir(path.join(outboxDir, "new"), { recursive: true });
  await fs.mkdir(choruzDir, { recursive: true });

  const sendScriptSrc = path.resolve(process.cwd(), "..", "..", "scripts", "choruz-send.sh");
  const sendScriptDst = path.join(choruzDir, "send");
  try {
    await fs.copyFile(sendScriptSrc, sendScriptDst);
    await fs.chmod(sendScriptDst, 0o755);
  } catch {
    await fs.writeFile(sendScriptDst, `#!/bin/bash
set -euo pipefail
[ $# -eq 0 ] && { echo "Usage: .choruz/send '{...}'" >&2; exit 1; }
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WS="$(dirname "$SCRIPT_DIR")"
if [[ "\${CHORUZ_OUTBOX_DIR:-}" = /* ]]; then
  OB="$CHORUZ_OUTBOX_DIR"
else
  OB="$WS/.choruz-outbox"
fi
mkdir -p "$OB/tmp" "$OB/new"
TMP=$(mktemp "$OB/tmp/cmd-XXXXXX")
echo "$1" > "$TMP"
LOCK="$OB/.lock"
until mkdir "$LOCK" 2>/dev/null; do sleep 0.01; done
cleanup_lock() { rmdir "$LOCK" 2>/dev/null || true; }
trap cleanup_lock EXIT
SEQ_FILE="$OB/.seq"
SEQ="0"
if [ -f "$SEQ_FILE" ]; then read -r SEQ < "$SEQ_FILE" || SEQ="0"; fi
SEQ_NUM=$((10#$SEQ + 1))
SEQ_PADDED=$(printf "%020d" "$SEQ_NUM")
printf "%s\\n" "$SEQ_PADDED" > "$SEQ_FILE"
mv "$TMP" "$OB/new/cmd-\${SEQ_PADDED}-$(basename "$TMP").json"
`, "utf-8");
    await fs.chmod(sendScriptDst, 0o755);
  }
}

export async function provisionAgent(
  input: AgentProvisioningInput,
  deps: AgentProvisioningDeps = defaultDeps,
): Promise<ProvisionResponse> {
  const completedSteps: Partial<Record<AgentProvisioningStepName, JsonValue>> = {};
  const body = input.body;
  const agentName = body.name.trim();
  const isWebhookDriver = body.driver_type === "webhook_agent";
  const harnessAccount = await resolveHarnessAccount(body, deps);
  const roleTemplateProvenance =
    input.roleTemplateProvenance ?? roleTemplateProvenanceFromRequestBody(body);

  async function step<T extends JsonValue>(
    stepName: AgentProvisioningStepName,
    action: () => Promise<{ value: T; record?: JsonValue }>,
  ): Promise<T> {
    const stepKey = buildStepKey(input.jobContext, stepName);
    if (input.jobContext && input.stepRecorder) {
      const existing = await input.stepRecorder.readStep<T>({
        key: stepKey,
        step: stepName,
        ...input.jobContext,
      });
      if (existing !== null) {
        completedSteps[stepName] = existing;
        return existing;
      }
    }

    try {
      const { value, record } = await action();
      const stored = (record ?? value) as T;
      if (input.jobContext && input.stepRecorder) {
        await input.stepRecorder.recordStep({
          key: stepKey,
          step: stepName,
          ...input.jobContext,
          output: stored,
        });
      }
      completedSteps[stepName] = stored;
      return value;
    } catch (error) {
      throw new AgentProvisioningError({
        step: stepName,
        message: error instanceof Error ? error.message : "Provisioning failed",
        completedSteps,
      });
    }
  }

  const principalResult = await step("create_principal", async () => {
    const { principal: agent, secret } = await deps.createAgent(
      input.sessionToken,
      input.actorId,
      agentName,
      [...AGENT_SCOPES],
      body.workspace_id,
      body.channel_visibility,
    );
    return {
      value: { agent, secret } as unknown as JsonValue,
      record: { agent: replaySafePrincipal(agent) } as unknown as JsonValue,
    };
  }) as unknown as CreatePrincipalOutput;
  const agent = principalResult.agent;
  let agentSecret = principalResult.secret;

  if (!agentSecret) {
    const rotated = await deps.rotateAgentSecret(input.sessionToken, input.actorId, agent.id);
    agentSecret = rotated.secret;
    await deps.persistAgentToken(agent.id, agentSecret);
    completedSteps.persist_token = { agentId: agent.id, persisted: true };
  } else {
    await step("persist_token", async () => {
      await deps.persistAgentToken(agent.id, agentSecret!);
      return { value: { agentId: agent.id, persisted: true } };
    });
  }

  const workspace = await step("create_workspace", async () => {
    const result = isWebhookDriver
      ? await createWebhookWorkspace(agent.id)
      : await createLocalWorkspace(body, agentName, input.jobContext);
    return { value: result as unknown as JsonValue };
  }) as unknown as WorkspaceResult;

  if (!isWebhookDriver) {
    await step("write_instructions", async () => {
      const instructionsFile = instructionFileForDriver(body.driver_type);
      const fullInstructions = await buildInstructionsFromTemplate(
        agentName,
        body.instructions ?? "",
        body.driver_type,
      );
      await fs.writeFile(path.join(workspace.workspacePath, instructionsFile), fullInstructions, "utf-8");
      return {
        value: {
          workspacePath: workspace.workspacePath,
          file: instructionsFile,
          choruzManaged: true,
        },
      };
    });

    await step("copy_skills", async () => {
      const copied = await copySelectedSkills(body.skill_paths, workspace.workspacePath);
      return { value: { workspacePath: workspace.workspacePath, copied } };
    });
  }

  await step("install_outbox_helper", async () => {
    await installOutboxHelper(workspace.workspacePath);
    return {
      value: {
        workspacePath: workspace.workspacePath,
        choruzManagedFiles: [".choruz/send", ".choruz-outbox"],
      },
    };
  });

  const conversation = await step("create_direct_conversation", async () => {
    const created = await deps.createDirectConversation(
      input.sessionToken,
      input.actorId,
      agent.id,
      body.workspace_id,
    );
    return { value: created as unknown as JsonValue };
  }) as unknown as Conversation;

  const binding = await step("create_runtime_binding", async () => {
    const created = await deps.createRuntimeBinding(
      input.sessionToken,
      input.actorId,
      conversation.id,
      agent.id,
      bindingDriverType(body.driver_type),
      workspace.workspacePath,
      {
        gitWorktreePath: workspace.gitWorktreePath,
        configJson: runtimeBindingConfig(body, agentName, harnessAccount),
      },
    );
    return {
      value: created as unknown as JsonValue,
      record: replaySafeRuntimeBinding(created) as unknown as JsonValue,
    };
  }) as unknown as RuntimeBinding;

  let webhookSecret: string | undefined;
  if (isWebhookDriver) {
    const webhookResult = await step("register_webhook", async () => {
      const secret = await registerWebhook(input, deps, agent.id);
      webhookSecret = secret;
      return {
        value: { webhookSecret: secret } as unknown as JsonValue,
        record: {
          agentId: agent.id,
          registered: true,
          eventTypes: ["app_mention"],
        } as unknown as JsonValue,
      };
    }) as unknown as { webhookSecret?: string };
    webhookSecret = webhookResult.webhookSecret ?? webhookSecret ?? body.webhook_secret?.trim();
    if (!webhookSecret && !input.jobContext?.allowWebhookRegistrationReplayWithoutOutput) {
      throw new AgentProvisioningError({
        step: "register_webhook",
        message: "Webhook was already registered, but its generated signing secret cannot be replayed. Retry with a caller-provided webhook_secret or rotate the webhook secret.",
        completedSteps,
      });
    }
  }

  if (roleTemplateProvenance && input.provenanceWriter) {
    await step("store_role_template_provenance", async () => {
      await input.provenanceWriter?.({
        agentPrincipalId: agent.id,
        roleTemplateId: roleTemplateProvenance.roleTemplateId,
        roleTemplateVersion: roleTemplateProvenance.roleTemplateVersion,
        instructionStatus: roleTemplateProvenance.instructionStatus,
        setupSummary: roleTemplateProvenance.setupSummary ?? {},
        workspaceMode: roleTemplateProvenance.workspaceMode,
        selectedSkills: roleTemplateProvenance.selectedSkills ?? [],
        originatingJobId: input.jobContext?.jobId ?? null,
      });
      return {
        value: {
          agentId: agent.id,
          roleTemplateId: roleTemplateProvenance.roleTemplateId,
          roleTemplateVersion: roleTemplateProvenance.roleTemplateVersion,
        },
      };
    });
  }

  return {
    agent,
    secret: agentSecret ?? "",
    conversation,
    binding,
    workspace_path: workspace.workspacePath,
    ...(webhookSecret ? { webhook_secret: webhookSecret } : {}),
  };
}

export const defaultRoleTemplateProvenanceWriter: RoleTemplateProvenanceWriter = async (input) => {
  const store = createGroupProvisioningStore(await postgresQueryClient());
  await store.insertAgentTemplateInstance({
    agentPrincipalId: input.agentPrincipalId,
    roleTemplateId: input.roleTemplateId,
    roleTemplateVersion: input.roleTemplateVersion,
    instructionStatus: input.instructionStatus,
    setupSummary: input.setupSummary,
    workspaceMode: input.workspaceMode,
    selectedSkills: input.selectedSkills,
    originatingJobId: input.originatingJobId,
  });
};

export function roleTemplateProvenanceFromRequestBody(
  body: ProvisionRequestBody,
): RoleTemplateProvenanceInput | null {
  const metadata = body.template_metadata;
  if (!metadata || metadata.mode !== "role_template") return null;
  return {
    roleTemplateId: metadata.roleTemplateId,
    roleTemplateVersion: metadata.roleTemplateVersion,
    instructionStatus: metadata.instructionStatus,
    setupSummary: metadata.setupSummary as JsonValue,
    workspaceMode: metadata.workspaceMode,
    selectedSkills: metadata.selectedSkills,
  };
}

function buildStepKey(
  context: AgentProvisioningJobContext | undefined,
  stepName: AgentProvisioningStepName,
): string {
  const safeStepName = stepKeySegment(stepName);
  if (!context) return safeStepName;
  return `${context.jobId}:${context.roleSlotId}:${context.idempotencyKey}:${safeStepName}`;
}

function stepKeySegment(stepName: AgentProvisioningStepName): string {
  return stepName === "persist_token" ? "persist_agent_auth" : stepName;
}

async function createWebhookWorkspace(agentId: string): Promise<WorkspaceResult> {
  const webhookHome = path.join(runtimeDir(), "webhook", agentId);
  await fs.mkdir(webhookHome, { recursive: true });
  return {
    workspacePath: webhookHome,
    gitWorktreePath: null,
    mode: "webhook",
    generatedByChoruz: true,
  };
}

async function createLocalWorkspace(
  body: ProvisionRequestBody,
  agentName: string,
  jobContext?: AgentProvisioningJobContext,
): Promise<WorkspaceResult> {
  if (body.workspace_path?.trim()) {
    const workspacePath = path.resolve(body.workspace_path.trim());
    await fs.mkdir(workspacePath, { recursive: true });
    return {
      workspacePath,
      gitWorktreePath: null,
      mode: "custom",
      generatedByChoruz: false,
    };
  }

  const sanitized = sanitizeName(agentName);
  const uniqueSuffix = jobContext
    ? crypto
        .createHash("sha256")
        .update(`${jobContext.jobId}:${jobContext.roleSlotId}:${jobContext.idempotencyKey}`)
        .digest("hex")
        .slice(0, 8)
    : crypto.randomUUID().slice(0, 8);
  const workspacePath = path.join(
    runtimeDir(),
    "workspaces",
    `${sanitized}-${uniqueSuffix}`,
    "workspace",
  );

  let gitWorktreePath: string | null = null;
  const repoPath = process.env.CHORUZ_GIT_REPO_PATH;
  if (repoPath) {
    const branchName = `choruz/${sanitized}-${uniqueSuffix}`;
    try {
      execFileSync("git", ["-C", repoPath, "worktree", "add", workspacePath, "-b", branchName], {
        stdio: "pipe",
      });
      gitWorktreePath = workspacePath;
    } catch (err) {
      console.error(
        "git worktree add failed, falling back to plain directory:",
        err instanceof Error ? err.message : err,
      );
      await fs.mkdir(workspacePath, { recursive: true });
    }
  } else {
    await fs.mkdir(workspacePath, { recursive: true });
  }

  return {
    workspacePath,
    gitWorktreePath,
    mode: "generated",
    generatedByChoruz: true,
  };
}

export function instructionFileForDriver(driverType: string): "CLAUDE.md" | "AGENTS.md" {
  // Imported Claude sessions use the non-provisionable `claude_print`
  // driver, but retain Claude Code's native instruction-file convention.
  if (driverType === "claude_print") return "CLAUDE.md";
  const entry = (DRIVER_FILES as Record<
    string,
    { instructions: "CLAUDE.md" | "AGENTS.md" }
  >)[driverType];
  if (!entry) {
    throw new Error(`Unsupported driver type: ${driverType}`);
  }
  return entry.instructions;
}

async function copySelectedSkills(skillPaths: string[] | undefined, workspacePath: string): Promise<string[]> {
  if (!Array.isArray(skillPaths) || skillPaths.length === 0) return [];
  const skillsDir = path.join(workspacePath, ".claude", "skills");
  await fs.mkdir(skillsDir, { recursive: true });

  const copied: string[] = [];
  const home = process.env.HOME;
  for (const sp of skillPaths) {
    const resolved = path.resolve(sp);
    if (!home || (resolved !== home && !resolved.startsWith(home + "/"))) {
      continue;
    }
    try {
      const stat = await fs.stat(resolved);
      const basename = path.basename(resolved);
      if (stat.isDirectory()) {
        await copyDirRecursive(resolved, path.join(skillsDir, basename));
        copied.push(basename);
      } else if (stat.isFile()) {
        const content = await fs.readFile(resolved, "utf-8");
        await fs.writeFile(path.join(skillsDir, basename), content, "utf-8");
        copied.push(basename);
      }
    } catch {
      // Preserve previous route behavior: skip unreadable skill entries.
    }
  }
  return copied;
}

function bindingDriverType(driverType: AgentProvisioningDriverType): string {
  return driverType;
}

function replaySafePrincipal(agent: PrincipalWithSensitiveFields): Principal {
  const { secret_hash: _secretHash, ...safeAgent } = agent;
  return safeAgent;
}

function replaySafeRuntimeBinding(binding: RuntimeBinding): RuntimeBindingCheckpoint {
  const {
    external_session_id: _externalSessionId,
    external_thread_id: _externalThreadId,
    ...safeBinding
  } = binding;
  return safeBinding;
}

function runtimeBindingConfig(
  body: ProvisionRequestBody,
  agentName: string,
  harnessAccount: HarnessAccount | null,
): Record<string, unknown> {
  if (body.driver_type === "webhook_agent") {
    return {
      is_primary: true,
      webhook_url: body.webhook_url,
      mention_aliases: [agentName],
    };
  }

  const binaryPath = {
    claude_terminal: process.env.CHORUZ_CLAUDE_BINARY || "claude",
    codex_exec: process.env.CHORUZ_CODEX_BINARY || "codex",
    codex_app_server: process.env.CHORUZ_CODEX_BINARY || "codex",
    codex_terminal: process.env.CHORUZ_CODEX_BINARY || "codex",
    pi_terminal: process.env.CHORUZ_PI_BINARY || "pi",
    grok_terminal: process.env.CHORUZ_GROK_BINARY || "grok",
    opencode_terminal: process.env.CHORUZ_OPENCODE_BINARY || "opencode",
    mathcode_terminal: process.env.CHORUZ_MATHCODE_BINARY || "mathcode",
  }[body.driver_type];
  return {
    is_primary: true,
    binary_path: binaryPath,
    original_driver: body.driver_type,
    mention_aliases: [agentName],
    ...(body.model?.trim() ? { model: body.model.trim() } : {}),
    ...(body.runtime_host_id?.trim()
      ? { runtime_host_id: body.runtime_host_id.trim() }
      : {}),
    ...(harnessAccount
      ? {
          harness_account_id: harnessAccount.id,
          harness_account_name: harnessAccount.name,
          harness_account_profile_kind: harnessAccount.profileKind,
        }
      : {}),
  };
}

/**
 * The chosen account, validated, or the device's default account when none
 * was chosen for a Claude Code or Codex agent.
 */
async function resolveHarnessAccount(
  body: ProvisionRequestBody,
  deps: Pick<AgentProvisioningDeps, "getHarnessAccount" | "defaultHarnessAccount">,
): Promise<HarnessAccount | null> {
  if (!body.harness_account_id) {
    if (body.driver_type !== "claude_terminal" && body.driver_type !== "codex_terminal") return null;
    if (!body.workspace_id) return null;
    return deps.defaultHarnessAccount({
      companyId: body.workspace_id,
      runtimeHostId: body.runtime_host_id?.trim() || null,
      driverType: body.driver_type,
      ...(body.model?.trim() ? { model: body.model.trim() } : {}),
    });
  }
  if (!body.workspace_id) throw new Error("A company is required when selecting a harness account");
  const account = await deps.getHarnessAccount(body.harness_account_id, body.workspace_id);
  if (!account) throw new Error("The selected harness account does not exist in this company");
  if (account.status !== "active") throw new Error("The selected harness account must be probed and active");
  if (account.driverType !== body.driver_type) throw new Error("The selected account belongs to a different harness");
  if ((account.runtimeHostId ?? "") !== (body.runtime_host_id?.trim() ?? "")) {
    throw new Error("The selected account belongs to a different runtime device");
  }
  if (body.model?.trim() && !account.models.some((model) => model.id === body.model?.trim())) {
    throw new Error("The selected model is not available to this harness account");
  }
  return account;
}

async function registerWebhook(
  input: AgentProvisioningInput,
  deps: AgentProvisioningDeps,
  agentId: string,
): Promise<string> {
  const gatewayBase =
    process.env.CHORUZ_API_BASE_URL?.trim()
    || process.env.CHORUZ_API_URL?.trim()
    || `http://127.0.0.1:${process.env.CHORUZ_API_PORT ?? "3000"}`;
  const response = await deps.fetch(
    `${gatewayBase}/v1/principals/${encodeURIComponent(agentId)}/event-webhook`,
    {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${input.sessionToken}`,
      },
      body: JSON.stringify({
        actor_id: input.actorId,
        url: input.body.webhook_url,
        event_types: ["app_mention"],
        ...(input.body.webhook_secret ? { secret: input.body.webhook_secret } : {}),
      }),
    },
  );
  if (!response.ok) {
    const errText = await response.text().catch(() => response.statusText);
    throw new Error(`Failed to register webhook: ${response.status} ${errText}`);
  }
  const config = (await response.json()) as { webhook_secret: string };
  return config.webhook_secret;
}
