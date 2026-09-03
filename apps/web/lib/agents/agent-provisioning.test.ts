import { mkdtemp, rm } from "node:fs/promises";
import { promises as fs } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  AgentProvisioningError,
  buildInstructionsFromTemplate,
  instructionFileForDriver,
  provisionAgent,
  roleTemplateProvenanceFromRequestBody,
  templateFileForDriver,
  validateProvisionRequestBody,
  type AgentProvisioningDeps,
  type AgentProvisioningStepRecord,
  type AgentProvisioningStepRecorder,
} from "./agent-provisioning";
import type { Conversation, Principal, RuntimeBinding } from "../api/choruz-api";
import type { HarnessAccount } from "./harness-accounts";
import { sanitizeProvisioningJson, type JsonValue } from "../groups/group-provisioning-store";

class MemoryStepRecorder implements AgentProvisioningStepRecorder {
  readonly records = new Map<string, JsonValue>();

  async readStep<T extends JsonValue>(
    input: Omit<AgentProvisioningStepRecord, "output">,
  ): Promise<T | null> {
    return (this.records.get(input.key) as T | undefined) ?? null;
  }

  async recordStep<T extends JsonValue>(record: AgentProvisioningStepRecord<T>): Promise<void> {
    sanitizeProvisioningJson({ [record.key]: record.output } as unknown as JsonValue, "stepResultsJson.agentSteps");
    this.records.set(record.key, record.output);
  }
}

describe("provisionAgent", () => {
  const originalRuntimeDir = process.env.CHORUZ_RUNTIME_DIR;
  const originalHome = process.env.HOME;
  const originalClaudeBinary = process.env.CHORUZ_CLAUDE_BINARY;
  const runtimeDirs: string[] = [];

  afterEach(async () => {
    vi.restoreAllMocks();
    if (originalRuntimeDir === undefined) delete process.env.CHORUZ_RUNTIME_DIR;
    else process.env.CHORUZ_RUNTIME_DIR = originalRuntimeDir;
    if (originalHome === undefined) delete process.env.HOME;
    else process.env.HOME = originalHome;
    if (originalClaudeBinary === undefined) delete process.env.CHORUZ_CLAUDE_BINARY;
    else process.env.CHORUZ_CLAUDE_BINARY = originalClaudeBinary;
    await Promise.all(runtimeDirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
  });

  it("reuses recorded step outputs when retrying after a mid-provisioning failure", async () => {
    const runtimeDir = await mkdtemp(path.join(tmpdir(), "choruz-agent-provisioning-"));
    runtimeDirs.push(runtimeDir);
    process.env.CHORUZ_RUNTIME_DIR = runtimeDir;

    const recorder = new MemoryStepRecorder();
    const calls = {
      createAgent: 0,
      rotateAgentSecret: 0,
      createDirectConversation: 0,
      createRuntimeBinding: 0,
      persistAgentToken: 0,
    };
    let failBinding = true;
    const deps = fakeDeps({
      createAgent: async () => {
        calls.createAgent += 1;
        return { principal: principalWithSecretHash("agent-1", "Helper"), secret: "agent-secret-1" };
      },
      rotateAgentSecret: async () => {
        calls.rotateAgentSecret += 1;
        return { principal: principal("agent-1", "Helper"), secret: "rotated-agent-secret-1" };
      },
      persistAgentToken: async () => {
        calls.persistAgentToken += 1;
      },
      createDirectConversation: async () => {
        calls.createDirectConversation += 1;
        return conversation("conversation-1", "agent-1");
      },
      createRuntimeBinding: async () => {
        calls.createRuntimeBinding += 1;
        if (failBinding) throw new Error("binding service unavailable");
        return binding("binding-1", "conversation-1", "agent-1", path.join(runtimeDir, "workspaces", "helper"));
      },
    });

    const input = {
      sessionToken: "session-token",
      actorId: "human-1",
      body: {
        name: "Helper",
        driver_type: "claude_terminal" as const,
        instructions: "Help with the task.",
      },
      jobContext: {
        jobId: "job-1",
        roleSlotId: "slot-a",
        idempotencyKey: "idem-1",
      },
      stepRecorder: recorder,
    };

    await expect(provisionAgent(input, deps)).rejects.toMatchObject({
      detail: {
        step: "create_runtime_binding",
        message: "binding service unavailable",
      },
    });
    expect(recorder.records.get("job-1:slot-a:idem-1:create_principal")).toEqual({
      agent: principal("agent-1", "Helper"),
    });
    expect(recorder.records.get("job-1:slot-a:idem-1:persist_agent_auth")).toEqual({
      agentId: "agent-1",
      persisted: true,
    });
    expect([...recorder.records.keys()]).not.toContain("job-1:slot-a:idem-1:persist_token");

    expect(calls).toEqual({
      createAgent: 1,
      rotateAgentSecret: 0,
      createDirectConversation: 1,
      createRuntimeBinding: 1,
      persistAgentToken: 1,
    });

    failBinding = false;
    const result = await provisionAgent(input, deps);

    expect(result.agent.id).toBe("agent-1");
    expect(result.secret).toBe("rotated-agent-secret-1");
    expect(result.conversation.id).toBe("conversation-1");
    expect(result.binding.id).toBe("binding-1");
    const recordedBinding = recorder.records.get("job-1:slot-a:idem-1:create_runtime_binding") as Record<string, unknown>;
    expect(recordedBinding).toEqual({
      id: "binding-1",
      workspace_id: "workspace-1",
      conversation_id: "conversation-1",
      conversation_name: "Direct",
      conversation_type: "direct",
      agent_principal_id: "agent-1",
      agent_name: "Helper",
      driver_type: "claude_terminal",
      workspace_path: result.binding.workspace_path,
      git_worktree_path: null,
      last_event_cursor: 0,
      last_acked_event_cursor: 0,
      last_seen_server_seq: 0,
      state: "idle",
      last_error: null,
      updated_at: "2026-01-01T00:00:00Z",
    });
    expect(recordedBinding.external_session_id).toBeUndefined();
    expect(recordedBinding.external_thread_id).toBeUndefined();
    expect(result.workspace_path).toBe(path.join(runtimeDir, "workspaces", "helper-7b7a80cc", "workspace"));
    expect(calls).toEqual({
      createAgent: 1,
      rotateAgentSecret: 1,
      createDirectConversation: 1,
      createRuntimeBinding: 2,
      persistAgentToken: 2,
    });
  });

  it("rotates and persists a secret when retrying after the principal step was recorded before token persistence", async () => {
    const runtimeDir = await mkdtemp(path.join(tmpdir(), "choruz-agent-provisioning-"));
    runtimeDirs.push(runtimeDir);
    process.env.CHORUZ_RUNTIME_DIR = runtimeDir;

    const recorder = new MemoryStepRecorder();
    await recorder.recordStep({
      key: "job-2:slot-a:idem-2:create_principal",
      step: "create_principal",
      jobId: "job-2",
      roleSlotId: "slot-a",
      idempotencyKey: "idem-2",
      output: { agent: principal("agent-2", "Recoverable") } as unknown as JsonValue,
    });

    const calls = {
      createAgent: 0,
      rotateAgentSecret: 0,
      persistAgentToken: 0,
    };
    const persistedSecrets: string[] = [];
    const deps = fakeDeps({
      createAgent: async () => {
        calls.createAgent += 1;
        return { principal: principal("duplicate-agent", "Recoverable"), secret: "duplicate-secret" };
      },
      rotateAgentSecret: async () => {
        calls.rotateAgentSecret += 1;
        return { principal: principal("agent-2", "Recoverable"), secret: "rotated-secret" };
      },
      persistAgentToken: async (_agentId, secret) => {
        calls.persistAgentToken += 1;
        persistedSecrets.push(secret);
      },
      createDirectConversation: async () => conversation("conversation-2", "agent-2"),
      createRuntimeBinding: async (_sessionToken, _actorId, conversationId, agentId, _driverType, workspacePath) =>
        binding("binding-2", conversationId, agentId, workspacePath),
    });

    const result = await provisionAgent({
      sessionToken: "session-token",
      actorId: "human-1",
      body: {
        name: "Recoverable",
        driver_type: "claude_terminal",
        instructions: "Help with the task.",
      },
      jobContext: {
        jobId: "job-2",
        roleSlotId: "slot-a",
        idempotencyKey: "idem-2",
      },
      stepRecorder: recorder,
    }, deps);

    expect(result.agent.id).toBe("agent-2");
    expect(result.secret).toBe("rotated-secret");
    expect(calls).toEqual({
      createAgent: 0,
      rotateAgentSecret: 1,
      persistAgentToken: 1,
    });
    expect(persistedSecrets).toEqual(["rotated-secret"]);
  });

  it("passes internal channel visibility through delegated provisioning", async () => {
    const runtimeDir = await mkdtemp(path.join(tmpdir(), "choruz-agent-provisioning-"));
    runtimeDirs.push(runtimeDir);
    process.env.CHORUZ_RUNTIME_DIR = runtimeDir;
    const createAgentMock = vi.fn(async () => ({
      principal: principal("agent-internal", "Internal Helper"),
      secret: "agent-secret",
    }));
    const deps = fakeDeps({ createAgent: createAgentMock });

    await provisionAgent({
      sessionToken: "session-token",
      actorId: "human-1",
      body: {
        name: "Internal Helper",
        driver_type: "claude_terminal",
        instructions: "Help privately.",
        channel_visibility: "internal",
      },
    }, deps);

    expect(createAgentMock).toHaveBeenCalledWith(
      "session-token",
      "human-1",
      "Internal Helper",
      ["messages:read", "messages:write", "events:read"],
      undefined,
      "internal",
    );
  });

  it("stores a selected model in the idle runtime binding without starting a Harness", async () => {
    const runtimeDir = await mkdtemp(path.join(tmpdir(), "choruz-agent-provisioning-"));
    runtimeDirs.push(runtimeDir);
    process.env.CHORUZ_RUNTIME_DIR = runtimeDir;
    const createRuntimeBindingMock = vi.fn(async (
      _sessionToken: string,
      _actorId: string,
      conversationId: string,
      agentId: string,
      _driverType: string,
      workspacePath: string,
    ) => binding("binding-model", conversationId, agentId, workspacePath));

    const result = await provisionAgent({
      sessionToken: "session-token",
      actorId: "human-1",
      body: {
        name: "Model Helper",
        driver_type: "codex_exec",
        model: "gpt-5.6-codex",
        instructions: "Help with the implementation.",
      },
    }, fakeDeps({ createRuntimeBinding: createRuntimeBindingMock }));

    expect(result.binding.state).toBe("idle");
    expect(result.binding.external_session_id).toBeNull();
    expect(createRuntimeBindingMock).toHaveBeenCalledWith(
      "session-token",
      "human-1",
      "conversation-default",
      "agent-default",
      "codex_exec",
      expect.any(String),
      expect.objectContaining({ configJson: expect.objectContaining({ model: "gpt-5.6-codex" }) }),
    );
  });

  it("binds only a verified model from the selected account on the selected device", async () => {
    const runtimeDir = await mkdtemp(path.join(tmpdir(), "choruz-agent-provisioning-"));
    runtimeDirs.push(runtimeDir);
    process.env.CHORUZ_RUNTIME_DIR = runtimeDir;
    const createRuntimeBindingMock = vi.fn(async (
      _sessionToken: string,
      _actorId: string,
      conversationId: string,
      agentId: string,
      _driverType: string,
      workspacePath: string,
    ) => binding("binding-account", conversationId, agentId, workspacePath));
    const account = verifiedAccount();

    await provisionAgent({
      sessionToken: "session-token",
      actorId: "human-1",
      body: {
        name: "Account Helper",
        driver_type: "codex_terminal",
        model: "gpt-5.6-sol",
        harness_account_id: account.id,
        workspace_id: "workspace-1",
        instructions: "Help with the implementation.",
      },
    }, fakeDeps({
      createRuntimeBinding: createRuntimeBindingMock,
      getHarnessAccount: vi.fn(async () => account),
    }));

    expect(createRuntimeBindingMock).toHaveBeenCalledWith(
      "session-token",
      "human-1",
      "conversation-default",
      "agent-default",
      "codex_terminal",
      expect.any(String),
      expect.objectContaining({
        configJson: expect.objectContaining({
          model: "gpt-5.6-sol",
          harness_account_id: account.id,
          harness_account_name: "Codex work",
          harness_account_profile_kind: "isolated",
        }),
      }),
    );
  });

  it("binds the device's default account when none was chosen", async () => {
    const runtimeDir = await mkdtemp(path.join(tmpdir(), "choruz-agent-provisioning-"));
    runtimeDirs.push(runtimeDir);
    process.env.CHORUZ_RUNTIME_DIR = runtimeDir;
    const createRuntimeBindingMock = vi.fn(async (
      _sessionToken: string,
      _actorId: string,
      conversationId: string,
      agentId: string,
      _driverType: string,
      workspacePath: string,
    ) => binding("binding-default-account", conversationId, agentId, workspacePath));
    const account = { ...verifiedAccount(), name: "Codex login", profileKind: "default" as const };
    const defaultHarnessAccount = vi.fn(async () => account);

    await provisionAgent({
      sessionToken: "session-token",
      actorId: "human-1",
      body: {
        name: "Default Account Helper",
        driver_type: "codex_terminal",
        model: "gpt-5.6-sol",
        workspace_id: "workspace-1",
        runtime_host_id: "host-1",
        instructions: "Help with the implementation.",
      },
    }, fakeDeps({ createRuntimeBinding: createRuntimeBindingMock, defaultHarnessAccount }));

    expect(defaultHarnessAccount).toHaveBeenCalledWith({
      companyId: "workspace-1",
      runtimeHostId: "host-1",
      driverType: "codex_terminal",
      model: "gpt-5.6-sol",
    });
    expect(createRuntimeBindingMock).toHaveBeenCalledWith(
      "session-token",
      "human-1",
      "conversation-default",
      "agent-default",
      "codex_terminal",
      expect.any(String),
      expect.objectContaining({
        configJson: expect.objectContaining({
          harness_account_id: account.id,
          harness_account_name: "Codex login",
          harness_account_profile_kind: "default",
        }),
      }),
    );
  });

  it("launches without an account when the device's login has not verified", async () => {
    const createRuntimeBindingMock = vi.fn(async (
      _sessionToken: string,
      _actorId: string,
      conversationId: string,
      agentId: string,
      _driverType: string,
      workspacePath: string,
    ) => binding("binding-no-account", conversationId, agentId, workspacePath));
    const defaultHarnessAccount = vi.fn(async () => null);

    await provisionAgent({
      sessionToken: "session-token",
      actorId: "human-1",
      body: {
        name: "Unverified Login Helper",
        driver_type: "claude_terminal",
        workspace_id: "workspace-1",
        instructions: "Help.",
      },
    }, fakeDeps({ createRuntimeBinding: createRuntimeBindingMock, defaultHarnessAccount }));

    expect(defaultHarnessAccount).toHaveBeenCalledWith({
      companyId: "workspace-1",
      runtimeHostId: null,
      driverType: "claude_terminal",
    });
    expect(createRuntimeBindingMock).toHaveBeenCalledWith(
      "session-token",
      "human-1",
      "conversation-default",
      "agent-default",
      "claude_terminal",
      expect.any(String),
      expect.objectContaining({
        configJson: expect.not.objectContaining({ harness_account_id: expect.anything() }),
      }),
    );
  });

  it("rejects a model that the selected account did not report", async () => {
    const createAgentMock = vi.fn();
    await expect(provisionAgent({
      sessionToken: "session-token",
      actorId: "human-1",
      body: {
        name: "Wrong Model",
        driver_type: "codex_terminal",
        model: "unverified-model",
        harness_account_id: verifiedAccount().id,
        workspace_id: "workspace-1",
        instructions: "Help.",
      },
    }, fakeDeps({
      createAgent: createAgentMock,
      getHarnessAccount: vi.fn(async () => verifiedAccount()),
    }))).rejects.toThrow("not available to this harness account");
    expect(createAgentMock).not.toHaveBeenCalled();
  });

  it("returns sanitized partial failure details without bearer tokens or binary paths", async () => {
    const runtimeDir = await mkdtemp(path.join(tmpdir(), "choruz-agent-provisioning-"));
    runtimeDirs.push(runtimeDir);
    process.env.CHORUZ_RUNTIME_DIR = runtimeDir;
    process.env.CHORUZ_CLAUDE_BINARY = "/Users/alice/bin/claude-secret-location";

    const deps = fakeDeps({
      createAgent: async () => ({ principal: principal("agent-3", "No Leak"), secret: "agent-secret-3" }),
      persistAgentToken: async () => {},
      createDirectConversation: async () => conversation("conversation-3", "agent-3"),
      createRuntimeBinding: async () => {
        throw new Error("runtime down");
      },
    });

    try {
      await provisionAgent({
        sessionToken: "session-token",
        actorId: "human-1",
        body: {
          name: "No Leak",
          driver_type: "claude_terminal",
          instructions: "Help.",
        },
        jobContext: {
          jobId: "job-3",
          roleSlotId: "slot-a",
          idempotencyKey: "idem-3",
        },
        stepRecorder: new MemoryStepRecorder(),
      }, deps);
      throw new Error("expected provisioning to fail");
    } catch (error) {
      expect(error).toBeInstanceOf(AgentProvisioningError);
      const serialized = JSON.stringify((error as AgentProvisioningError).detail);
      expect(serialized).not.toContain("agent-secret-3");
      expect(serialized).not.toContain("session-token");
      expect(serialized).not.toContain("/Users/alice/bin/claude-secret-location");
    }
  });

  it("maps role-template request metadata into provisioning provenance input", () => {
    expect(roleTemplateProvenanceFromRequestBody({
      name: "Template Agent",
      driver_type: "codex_terminal",
      instructions: "Generated instructions.",
      template_metadata: {
        mode: "role_template",
        roleTemplateId: "backend-engineer",
        roleTemplateVersion: "1.0.0",
        instructionStatus: "template_default",
        setupSummary: {
          repository_path: "/repo",
        },
        selectedSkills: ["/skills/repo-navigation"],
        workspaceMode: "generated",
      },
    })).toEqual({
      roleTemplateId: "backend-engineer",
      roleTemplateVersion: "1.0.0",
      instructionStatus: "template_default",
      setupSummary: {
        repository_path: "/repo",
      },
      selectedSkills: ["/skills/repo-navigation"],
      workspaceMode: "generated",
    });
  });

  it("stores role-template provenance from request metadata after successful provisioning", async () => {
    const runtimeDir = await mkdtemp(path.join(tmpdir(), "choruz-agent-provisioning-"));
    runtimeDirs.push(runtimeDir);
    process.env.CHORUZ_RUNTIME_DIR = runtimeDir;

    const provenanceWriter = vi.fn(async () => {});
    const deps = fakeDeps({
      createAgent: async () => ({ principal: principal("agent-4", "Template Agent"), secret: "agent-secret-4" }),
      persistAgentToken: async () => {},
      createDirectConversation: async () => conversation("conversation-4", "agent-4"),
      createRuntimeBinding: async (_sessionToken, _actorId, conversationId, agentId, _driverType, workspacePath) =>
        binding("binding-4", conversationId, agentId, workspacePath),
    });

    await provisionAgent({
      sessionToken: "session-token",
      actorId: "human-1",
      body: {
        name: "Template Agent",
        driver_type: "codex_terminal",
        instructions: "Generated instructions.",
        template_metadata: {
          mode: "role_template",
          roleTemplateId: "backend-engineer",
          roleTemplateVersion: "1.0.0",
          instructionStatus: "customized",
          setupSummary: {
            repository_path: "/repo",
          },
          selectedSkills: ["/skills/repo-navigation"],
          workspaceMode: "generated",
        },
      },
      provenanceWriter,
    }, deps);

    expect(provenanceWriter).toHaveBeenCalledWith({
      agentPrincipalId: "agent-4",
      roleTemplateId: "backend-engineer",
      roleTemplateVersion: "1.0.0",
      instructionStatus: "customized",
      setupSummary: {
        repository_path: "/repo",
      },
      workspaceMode: "generated",
      selectedSkills: ["/skills/repo-navigation"],
      originatingJobId: null,
    });
  });

  it("does not silently omit generated webhook secrets when replaying a recorded webhook step", async () => {
    const runtimeDir = await mkdtemp(path.join(tmpdir(), "choruz-agent-provisioning-"));
    runtimeDirs.push(runtimeDir);
    process.env.CHORUZ_RUNTIME_DIR = runtimeDir;

    const recorder = new MemoryStepRecorder();
    const input = {
      sessionToken: "session-token",
      actorId: "human-1",
      body: {
        name: "Webhook Agent",
        driver_type: "webhook_agent" as const,
        webhook_url: "https://example.com/hook",
      },
      jobContext: {
        jobId: "job-webhook",
        roleSlotId: "slot-webhook",
        idempotencyKey: "idem-webhook",
      },
      stepRecorder: recorder,
    };
    const deps = fakeDeps({
      createAgent: async () => ({ principal: principal("agent-webhook", "Webhook Agent"), secret: "agent-secret-webhook" }),
      persistAgentToken: async () => {},
      createDirectConversation: async () => conversation("conversation-webhook", "agent-webhook"),
      createRuntimeBinding: async (_sessionToken, _actorId, conversationId, agentId, _driverType, workspacePath) =>
        binding("binding-webhook", conversationId, agentId, workspacePath),
      fetch: vi.fn(async () => new Response(JSON.stringify({ webhook_secret: "generated-webhook-secret" }))),
    });

    const first = await provisionAgent(input, deps);
    expect(first.webhook_secret).toBe("generated-webhook-secret");
    const outboxNewStat = await fs.stat(path.join(first.workspace_path, ".choruz-outbox", "new"));
    expect(outboxNewStat.isDirectory()).toBe(true);
    const sendStat = await fs.stat(path.join(first.workspace_path, ".choruz", "send"));
    expect(sendStat.mode & 0o111).not.toBe(0);

    await expect(provisionAgent(input, deps)).rejects.toMatchObject({
      detail: {
        step: "register_webhook",
        message: expect.stringContaining("generated signing secret cannot be replayed"),
      },
    });
  });

  it("allows opted-in job retries to replay generated webhook registration without a stored secret", async () => {
    const runtimeDir = await mkdtemp(path.join(tmpdir(), "choruz-agent-provisioning-"));
    runtimeDirs.push(runtimeDir);
    process.env.CHORUZ_RUNTIME_DIR = runtimeDir;

    const recorder = new MemoryStepRecorder();
    const input = {
      sessionToken: "session-token",
      actorId: "human-1",
      body: {
        name: "Webhook Agent",
        driver_type: "webhook_agent" as const,
        webhook_url: "https://example.com/hook",
      },
      jobContext: {
        jobId: "job-webhook-group",
        roleSlotId: "slot-webhook",
        idempotencyKey: "idem-webhook",
        allowWebhookRegistrationReplayWithoutOutput: true,
      },
      stepRecorder: recorder,
    };
    const deps = fakeDeps({
      createAgent: async () => ({ principal: principal("agent-webhook-group", "Webhook Agent"), secret: "agent-secret-webhook" }),
      persistAgentToken: async () => {},
      createDirectConversation: async () => conversation("conversation-webhook-group", "agent-webhook-group"),
      createRuntimeBinding: async (_sessionToken, _actorId, conversationId, agentId, _driverType, workspacePath) =>
        binding("binding-webhook-group", conversationId, agentId, workspacePath),
      fetch: vi.fn(async () => new Response(JSON.stringify({ webhook_secret: "generated-webhook-secret" }))),
    });

    const first = await provisionAgent(input, deps);
    const replayed = await provisionAgent(input, deps);

    expect(first.webhook_secret).toBe("generated-webhook-secret");
    expect(replayed).not.toHaveProperty("webhook_secret");
    expect(JSON.stringify([...recorder.records.values()])).not.toContain("generated-webhook-secret");
  });

  it("replays caller-provided webhook secrets without recording secret material", async () => {
    const runtimeDir = await mkdtemp(path.join(tmpdir(), "choruz-agent-provisioning-"));
    runtimeDirs.push(runtimeDir);
    process.env.CHORUZ_RUNTIME_DIR = runtimeDir;

    const recorder = new MemoryStepRecorder();
    const input = {
      sessionToken: "session-token",
      actorId: "human-1",
      body: {
        name: "Webhook Agent",
        driver_type: "webhook_agent" as const,
        webhook_url: "https://example.com/hook",
        webhook_secret: "caller-provided-secret",
      },
      jobContext: {
        jobId: "job-webhook-provided",
        roleSlotId: "slot-webhook",
        idempotencyKey: "idem-webhook",
      },
      stepRecorder: recorder,
    };
    const deps = fakeDeps({
      createAgent: async () => ({ principal: principal("agent-webhook-provided", "Webhook Agent"), secret: "agent-secret-webhook" }),
      persistAgentToken: async () => {},
      createDirectConversation: async () => conversation("conversation-webhook-provided", "agent-webhook-provided"),
      createRuntimeBinding: async (_sessionToken, _actorId, conversationId, agentId, _driverType, workspacePath) =>
        binding("binding-webhook-provided", conversationId, agentId, workspacePath),
      fetch: vi.fn(async () => new Response(JSON.stringify({ webhook_secret: "caller-provided-secret" }))),
    });

    await provisionAgent(input, deps);
    const replayed = await provisionAgent(input, deps);

    expect(replayed.webhook_secret).toBe("caller-provided-secret");
    expect(JSON.stringify([...recorder.records.values()])).not.toContain("caller-provided-secret");
  });

  it("validates webhook URLs while preserving caller-provided secret compatibility", async () => {
    expect(validateProvisionRequestBody({
      name: "Webhook Agent",
      driver_type: "webhook_agent",
      webhook_url: "http://",
    })).toBe("Field `webhook_url` must be a valid URL for webhook_agent.");

    expect(validateProvisionRequestBody({
      name: "Webhook Agent",
      driver_type: "webhook_agent",
      webhook_url: "https://example.com/hook",
      webhook_secret: "short",
    })).toBeNull();
    expect(validateProvisionRequestBody({
      name: "Model Agent",
      driver_type: "claude_terminal",
      instructions: "Help.",
      model: "bad\nmodel",
    })).toBe("Field `model` cannot contain control characters.");
    expect(validateProvisionRequestBody({
      name: "Webhook Agent",
      driver_type: "webhook_agent",
      webhook_url: "https://example.com/hook",
      model: "gpt-5",
    })).toBe("Field `model` is not accepted for webhook_agent.");
    expect(validateProvisionRequestBody({
      name: "Math Agent",
      driver_type: "mathcode_terminal",
      instructions: "Formalize and prove the theorem.",
      model: "gpt-5.6-sol",
    })).toBe("Field `model` is not accepted for mathcode_terminal.");
    expect(validateProvisionRequestBody({
      name: "Webhook Agent",
      driver_type: "webhook_agent",
      webhook_url: "https://example.com/hook",
      runtime_host_id: "host-west",
    })).toBe("Field `runtime_host_id` is not accepted for webhook_agent.");
    expect(validateProvisionRequestBody({
      name: "Webhook Agent",
      driver_type: "webhook_agent",
      webhook_url: "https://example.com/hook",
      runtime_host_id: 123 as unknown as string,
    })).toBe("Field `runtime_host_id` must be a non-empty string.");
    expect(validateProvisionRequestBody({
      name: "Account Agent",
      driver_type: "claude_terminal",
      instructions: "Help.",
      harness_account_id: "../../credentials",
    })).toBe("Field `harness_account_id` must be a valid account id.");
    expect(validateProvisionRequestBody({
      name: "Webhook Agent",
      driver_type: "webhook_agent",
      webhook_url: "https://example.com/hook",
      harness_account_id: "12345678-1234-1234-1234-123456789abc",
    })).toBe("Field `harness_account_id` is not accepted for webhook_agent.");
    expect(validateProvisionRequestBody({
      name: "Private Helper",
      driver_type: "claude_terminal",
      instructions: "Help privately.",
      channel_visibility: "internal",
    })).toBe("Field `channel_visibility` is not accepted by this route.");
    expect(validateProvisionRequestBody({
      name: "Private Helper",
      driver_type: "claude_terminal",
      instructions: "Help privately.",
      channel_visibility: "private" as "internal",
    }, { allowChannelVisibility: true })).toBe('Field `channel_visibility` must be "visible" or "internal".');
    expect(validateProvisionRequestBody({
      name: "App Server Helper",
      driver_type: "codex_app_server",
      instructions: "Help through Codex app-server semantics.",
    })).toBeNull();
    expect(validateProvisionRequestBody({
      name: "Idempotent Helper",
      driver_type: "codex_terminal",
      instructions: "Help once.",
      idempotency_key: "company:company-1:ai-manager",
    })).toBeNull();
    expect(validateProvisionRequestBody({
      name: "Invalid Key Helper",
      driver_type: "codex_terminal",
      instructions: "Do not start.",
      idempotency_key: "contains spaces",
    })).toContain("idempotency_key");
    expect(validateProvisionRequestBody({
      name: "Unsafe Model",
      driver_type: "codex_exec",
      instructions: "Help.",
      model: "--help",
    })).toBe("Field `model` cannot start with `-`.");
  });
});

function fakeDeps(overrides: Partial<AgentProvisioningDeps>): AgentProvisioningDeps {
  return {
    createAgent: vi.fn(async () => ({ principal: principal("agent-default", "Default"), secret: "secret" })),
    rotateAgentSecret: vi.fn(async () => ({ principal: principal("agent-default", "Default"), secret: "rotated" })),
    persistAgentToken: vi.fn(async () => {}),
    createDirectConversation: vi.fn(async () => conversation("conversation-default", "agent-default")),
    createRuntimeBinding: vi.fn(async (_sessionToken, _actorId, conversationId, agentId, _driverType, workspacePath) =>
      binding("binding-default", conversationId, agentId, workspacePath)),
    getHarnessAccount: vi.fn(async () => null),
    defaultHarnessAccount: vi.fn(async () => null),
    fetch: vi.fn(async () => new Response(JSON.stringify({ webhook_secret: "webhook-secret" }))),
    ...overrides,
  };
}

function verifiedAccount(): HarnessAccount {
  return {
    id: "12345678-1234-1234-1234-123456789abc",
    companyId: "workspace-1",
    runtimeHostId: null,
    driverType: "codex_terminal",
    name: "Codex work",
    profileKind: "isolated",
    subscriptionType: "team",
    status: "active",
    models: [{ id: "gpt-5.6-sol", label: "GPT-5.6 Sol" }],
    usage: { windows: [] },
    lastError: null,
    probedAt: "2026-09-01T00:00:00Z",
    createdAt: "2026-09-01T00:00:00Z",
    updatedAt: "2026-09-01T00:00:00Z",
  };
}

function principal(id: string, name: string): Principal {
  return {
    id,
    workspace_id: "workspace-1",
    principal_type: "agent",
    name,
    avatar_url: null,
    scopes: ["messages:read", "messages:write", "events:read"],
    disabled: false,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  };
}

function principalWithSecretHash(id: string, name: string): Principal {
  return {
    ...principal(id, name),
    secret_hash: "stored-hash",
  } as Principal;
}

function conversation(id: string, agentId: string): Conversation {
  return {
    id,
    workspace_id: "workspace-1",
    conversation_type: "direct",
    name: null,
    description: null,
    avatar_url: null,
    creator_id: "human-1",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    members: {
      "human-1": { principal_id: "human-1", joined_at: "2026-01-01T00:00:00Z" },
      [agentId]: { principal_id: agentId, joined_at: "2026-01-01T00:00:00Z" },
    },
  };
}

function binding(id: string, conversationId: string, agentId: string, workspacePath: string): RuntimeBinding {
  return {
    id,
    workspace_id: "workspace-1",
    conversation_id: conversationId,
    conversation_name: "Direct",
    conversation_type: "direct",
    agent_principal_id: agentId,
    agent_name: "Helper",
    driver_type: "claude_terminal",
    workspace_path: workspacePath,
    git_worktree_path: null,
    external_session_id: null,
    external_thread_id: null,
    last_event_cursor: 0,
    last_acked_event_cursor: 0,
    last_seen_server_seq: 0,
    state: "idle",
    last_error: null,
    updated_at: "2026-01-01T00:00:00Z",
  };
}

describe("templateFileForDriver", () => {
  it("returns the codex template for codex drivers", () => {
    expect(templateFileForDriver("codex_terminal")).toBe("agent-codex-md-template.md");
    expect(templateFileForDriver("codex_exec")).toBe("agent-codex-md-template.md");
    expect(templateFileForDriver("codex_app_server")).toBe("agent-codex-md-template.md");
    expect(templateFileForDriver("pi_terminal")).toBe("agent-codex-md-template.md");
    expect(templateFileForDriver("grok_terminal")).toBe("agent-codex-md-template.md");
    expect(templateFileForDriver("opencode_terminal")).toBe("agent-codex-md-template.md");
  });

  it("returns the claude template for the claude driver and unknown drivers", () => {
    expect(templateFileForDriver("claude_terminal")).toBe("agent-claude-md-template.md");
    expect(templateFileForDriver("webhook_agent")).toBe("agent-claude-md-template.md");
    expect(templateFileForDriver("")).toBe("agent-claude-md-template.md");
  });
});

describe("instructionFileForDriver", () => {
  it.each(["codex_terminal", "codex_exec", "pi_terminal", "grok_terminal", "opencode_terminal", "mathcode_terminal"])(
    "uses AGENTS.md for %s",
    (driverType) => {
      expect(instructionFileForDriver(driverType)).toBe("AGENTS.md");
    },
  );

  it("uses CLAUDE.md for Claude modes and rejects unknown drivers", () => {
    expect(instructionFileForDriver("claude_terminal")).toBe("CLAUDE.md");
    expect(instructionFileForDriver("claude_print")).toBe("CLAUDE.md");
    expect(() => instructionFileForDriver("future_driver")).toThrow(
      "Unsupported driver type: future_driver",
    );
  });
});

describe("buildInstructionsFromTemplate", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders the claude template by default and substitutes user instructions", async () => {
    const rendered = await buildInstructionsFromTemplate("Helper", "Help with the task.");
    expect(rendered).toContain("Claude-compatible Choruz runtime");
    expect(rendered).toMatch(/^<!-- choruz-bootstrap-version: 10 -->/);
    expect(rendered).toContain("<!-- choruz-role:start -->");
    expect(rendered).toContain("<!-- choruz-role:end -->");
    expect(rendered).not.toContain("{{AGENT_INSTRUCTIONS}}");
    expect(rendered).not.toContain("{{CHORUZ_CORE_PROTOCOL}}");
    expect(rendered).not.toContain("metadata.workflow");
    expect(rendered).toContain("[choruz-incoming] METADATA | BODY");
    expect(rendered).toContain("Mutable Command Result Envelopes");
    expect(rendered).toContain("Help with the task.");
  });

  it("renders the codex template for codex_terminal", async () => {
    const rendered = await buildInstructionsFromTemplate(
      "Helper",
      "Help with the task.",
      "codex_terminal",
    );
    expect(rendered).toContain("# Choruz Platform Agent");
    expect(rendered).toMatch(/^<!-- choruz-bootstrap-version: 10 -->/);
    expect(rendered).toContain("<!-- choruz-role:start -->");
    expect(rendered).toContain("<!-- choruz-role:end -->");
    expect(rendered).toContain("AGENTS.md");
    expect(rendered).not.toContain("Claude-compatible Choruz runtime");
    expect(rendered).not.toContain("Gemini CLI");
    expect(rendered).not.toContain("{{AGENT_INSTRUCTIONS}}");
    expect(rendered).toContain("Help with the task.");
  });

  it("renders the codex template for codex_exec", async () => {
    const rendered = await buildInstructionsFromTemplate(
      "Helper",
      "Help with the task.",
      "codex_exec",
    );
    expect(rendered).toContain("# Choruz Platform Agent");
    expect(rendered).not.toContain("Claude-compatible Choruz runtime");
    expect(rendered).not.toContain("Gemini CLI");
  });

  it("does not impose terminal-only behavior on app-server or webhook drivers", async () => {
    const appServer = await buildInstructionsFromTemplate(
      "Helper",
      "Help with the task.",
      "codex_app_server",
    );
    const webhook = await buildInstructionsFromTemplate(
      "Helper",
      "Help with the task.",
      "webhook_agent",
    );
    expect(appServer).toContain("Codex CLI/app-server");
    expect(webhook).toContain("Claude-compatible Choruz runtime");
    expect(appServer).not.toContain("terminal output");
    expect(webhook).not.toContain("terminal output");
  });

  it.each(["pi_terminal", "grok_terminal", "opencode_terminal"])(
    "renders the shared AGENTS.md template for %s",
    async (driverType) => {
      const rendered = await buildInstructionsFromTemplate(
        "Helper",
        "Help with the task.",
        driverType,
      );
      expect(rendered).toContain("AGENTS.md");
      expect(rendered).toContain("Pi Agent");
      expect(rendered).toContain("Grok Build");
      expect(rendered).toContain("OpenCode");
    },
  );

  it("falls back to the claude template for unknown drivers", async () => {
    const rendered = await buildInstructionsFromTemplate(
      "Helper",
      "Help with the task.",
      "some_future_driver",
    );
    expect(rendered).toContain("Claude-compatible Choruz runtime");
  });

  it("uses a default instructions paragraph when none are supplied", async () => {
    const rendered = await buildInstructionsFromTemplate("Helper", "", "claude_terminal");
    expect(rendered).toContain("You are Helper, an AI assistant on the Choruz platform.");
    expect(rendered).not.toContain("{{AGENT_INSTRUCTIONS}}");
  });

  it("throws when no template file can be read", async () => {
    const enoent = Object.assign(new Error("ENOENT"), { code: "ENOENT" });
    vi.spyOn(fs, "readFile").mockRejectedValue(enoent);
    await expect(
      buildInstructionsFromTemplate("Helper", "Help with the task.", "codex_terminal"),
    ).rejects.toThrow(/agent-codex-md-template\.md/);
  });

  it("does not hide a permission failure mixed with missing fragments", async () => {
    const missing = Object.assign(new Error("missing"), { code: "ENOENT" });
    const denied = Object.assign(new Error("permission denied"), { code: "EACCES" });
    vi.spyOn(fs, "readFile")
      .mockRejectedValueOnce(missing)
      .mockRejectedValueOnce(denied)
      .mockRejectedValue(missing);
    await expect(
      buildInstructionsFromTemplate("Helper", "Help with the task.", "codex_terminal"),
    ).rejects.toBe(denied);
  });
});
