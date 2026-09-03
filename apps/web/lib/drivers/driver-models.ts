import { spawn } from "node:child_process";

import { query, type ModelInfo } from "@anthropic-ai/claude-agent-sdk";

import { resolveDriverBinary } from "./driver-availability";
import type { DriverId } from "../groups/team-templates";

export { validateModelId } from "./driver-model-validation";

export type DriverModel = {
  id: string;
  label: string;
  description?: string;
  provider?: string;
  resolvedModel?: string;
  capabilities?: {
    effortLevels?: string[];
    adaptiveThinking?: boolean;
    fastMode?: boolean;
    autoMode?: boolean;
  };
};

export type DriverModelDiscovery = {
  driverId: DriverId;
  status: "available" | "auth_required" | "unavailable" | "unsupported";
  models: DriverModel[];
  message: string;
};

type CommandOutput = { stdout: string; stderr: string };
export type ModelCommandRunner = (
  binary: string,
  args: string[],
) => Promise<CommandOutput>;

type DriverModelDiscoveryOptions = {
  env?: Record<string, string | undefined>;
  runCommand?: ModelCommandRunner;
  discoverClaude?: (binary: string) => Promise<DriverModel[]>;
  discoverCodex?: (binary: string) => Promise<DriverModel[]>;
  bypassCache?: boolean;
};

const CACHE_TTL_MS = 60_000;
const FAILURE_CACHE_TTL_MS = 5_000;
const discoveryCache = new Map<
  DriverId,
  { expiresAt: number; value: DriverModelDiscovery }
>();
const inFlightDiscoveries = new Map<DriverId, Promise<DriverModelDiscovery>>();

export async function discoverDriverModels(
  driverId: DriverId,
  options: DriverModelDiscoveryOptions = {},
): Promise<DriverModelDiscovery> {
  if (driverId === "webhook_agent" || driverId === "mathcode_terminal") {
    return {
      driverId,
      status: "unsupported",
      models: [],
      message: driverId === "mathcode_terminal"
        ? "MathCode selects its model through its own configuration."
        : "External agents choose their model in the external service.",
    };
  }

  if (!options.bypassCache) {
    const cached = discoveryCache.get(driverId);
    if (cached && cached.expiresAt > Date.now()) return cached.value;
    const inFlight = inFlightDiscoveries.get(driverId);
    if (inFlight) return inFlight;
  }

  const discovery = discoverDriverModelsUncached(driverId, options);
  if (options.bypassCache) return discovery;

  inFlightDiscoveries.set(driverId, discovery);
  try {
    return await discovery;
  } finally {
    if (inFlightDiscoveries.get(driverId) === discovery) {
      inFlightDiscoveries.delete(driverId);
    }
  }
}

async function discoverDriverModelsUncached(
  driverId: Exclude<DriverId, "webhook_agent" | "mathcode_terminal">,
  options: DriverModelDiscoveryOptions,
): Promise<DriverModelDiscovery> {
  const binary = resolveDriverBinary(driverId, options.env ?? process.env);
  if (!binary) {
    return unavailable(driverId, "The selected harness has no configured binary.");
  }

  try {
    const runCommand = options.runCommand ?? runModelCommand;
    let models: DriverModel[];
    switch (driverId) {
      case "claude_terminal":
        models = await (options.discoverClaude ?? discoverClaudeModels)(binary);
        break;
      case "codex_exec":
      case "codex_terminal":
        models = await (options.discoverCodex ?? discoverCodexModels)(binary);
        break;
      case "pi_terminal": {
        const output = await runCommand(binary, ["--list-models"]);
        models = parsePiModels(output.stdout);
        break;
      }
      case "grok_terminal": {
        const output = await runCommand(binary, ["models"]);
        models = parseGrokModels(`${output.stdout}\n${output.stderr}`);
        break;
      }
      case "opencode_terminal": {
        const output = await runCommand(binary, ["models"]);
        models = parseOpenCodeModels(output.stdout);
        break;
      }
      default:
        models = [];
    }

    const discoveredModels = dedupeModels(models);
    const value: DriverModelDiscovery = discoveredModels.length
      ? {
          driverId,
          status: "available",
          models: discoveredModels,
          message: `${discoveredModels.length} models discovered from the installed harness.`,
        }
      : unavailable(driverId, "The harness returned no selectable models.");
    discoveryCache.set(driverId, {
      expiresAt: Date.now() + (value.status === "available" ? CACHE_TTL_MS : FAILURE_CACHE_TTL_MS),
      value,
    });
    return value;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const authRequired = /auth|login|oauth|credential|session expired|unauthorized/i.test(
      message,
    );
    const value: DriverModelDiscovery = {
      driverId,
      status: authRequired ? "auth_required" : "unavailable",
      models: [],
      message: authRequired
        ? "Sign in to this harness before scanning its models."
        : "Model discovery is unavailable. You can still enter an exact model ID.",
    };
    discoveryCache.set(driverId, {
      expiresAt: Date.now() + FAILURE_CACHE_TTL_MS,
      value,
    });
    return value;
  }
}

export function clearDriverModelDiscoveryCache(): void {
  discoveryCache.clear();
  inFlightDiscoveries.clear();
}

export function parsePiModels(output: string): DriverModel[] {
  const lines = output.split(/\r?\n/);
  const headerIndex = lines.findIndex((line) => /^provider\s+model\s+/i.test(line.trim()));
  if (headerIndex < 0) return [];
  return lines.slice(headerIndex + 1).flatMap((line) => {
    const match = line.trim().match(/^(\S+)\s+(\S+)\s+/);
    if (!match) return [];
    const [, provider, model] = match;
    return [{ id: `${provider}/${model}`, label: model, provider }];
  });
}

export function parseGrokModels(output: string): DriverModel[] {
  const marker = output.match(/Available models:\s*([\s\S]*)/i)?.[1] ?? "";
  return marker.split(/\r?\n/).flatMap((line) => {
    const match = line.match(/^\s*[*-]\s+([^\s(]+)(?:\s+\(default\))?/);
    if (!match) return [];
    const id = match[1];
    return [{ id, label: id, description: /\(default\)/.test(line) ? "Default" : undefined }];
  });
}

export function parseOpenCodeModels(output: string): DriverModel[] {
  return output.split(/\r?\n/).flatMap((line) => {
    const id = line.trim();
    if (!id || /\s/.test(id) || !id.includes("/")) return [];
    const provider = id.slice(0, id.indexOf("/"));
    return [{ id, label: id.slice(id.indexOf("/") + 1), provider }];
  });
}

export function parseCodexModelListResult(result: unknown): {
  models: DriverModel[];
  nextCursor: string | null;
} {
  if (!result || typeof result !== "object") return { models: [], nextCursor: null };
  const record = result as Record<string, unknown>;
  const rows = Array.isArray(record.data) ? record.data : [];
  const models = rows.flatMap((row) => {
    if (!row || typeof row !== "object") return [];
    const model = row as Record<string, unknown>;
    if (typeof model.id !== "string" || typeof model.displayName !== "string") return [];
    const efforts = Array.isArray(model.supportedReasoningEfforts)
      ? model.supportedReasoningEfforts.flatMap((effort) => {
          if (typeof effort === "string") return [effort];
          if (!effort || typeof effort !== "object") return [];
          const value = (effort as Record<string, unknown>).reasoningEffort;
          return typeof value === "string" ? [value] : [];
        })
      : [];
    return [{
      id: model.id,
      label: model.displayName,
      description: typeof model.description === "string" ? model.description : undefined,
      capabilities: efforts.length ? { effortLevels: efforts } : undefined,
    }];
  });
  return {
    models,
    nextCursor: typeof record.nextCursor === "string" ? record.nextCursor : null,
  };
}

async function discoverClaudeModels(binary: string): Promise<DriverModel[]> {
  let finishInput: ((result: IteratorResult<never>) => void) | undefined;
  const input: AsyncIterable<never> & AsyncIterator<never> = {
    [Symbol.asyncIterator]() {
      return this;
    },
    next() {
      return new Promise<IteratorResult<never>>((resolve) => {
        finishInput = resolve;
      });
    },
    return() {
      finishInput?.({ done: true, value: undefined as never });
      return Promise.resolve({ done: true, value: undefined as never });
    },
  };
  const session = query({
    prompt: input,
    options: {
      pathToClaudeCodeExecutable: binary,
      cwd: process.cwd(),
      settingSources: ["user", "project", "local"],
    },
  });
  try {
    const models = await withTimeout(
      session.supportedModels(),
      15_000,
      "Claude model discovery timed out",
    );
    return models.map(claudeModelInfo);
  } finally {
    await input.return?.();
    session.close();
  }
}

function claudeModelInfo(model: ModelInfo): DriverModel {
  const capabilities = {
    ...(model.supportedEffortLevels?.length
      ? { effortLevels: [...model.supportedEffortLevels] }
      : {}),
    ...(model.supportsAdaptiveThinking ? { adaptiveThinking: true } : {}),
    ...(model.supportsFastMode ? { fastMode: true } : {}),
    ...(model.supportsAutoMode ? { autoMode: true } : {}),
  };
  return {
    id: model.value,
    label: model.displayName,
    description: model.description,
    ...(model.resolvedModel ? { resolvedModel: model.resolvedModel } : {}),
    ...(Object.keys(capabilities).length ? { capabilities } : {}),
  };
}

async function discoverCodexModels(binary: string): Promise<DriverModel[]> {
  const maxOutputBytes = 8 * 1024 * 1024;
  const maxPages = 20;
  const maxModels = 2_000;
  return new Promise<DriverModel[]>((resolve, reject) => {
    const child = spawn(binary, ["app-server"], {
      stdio: ["pipe", "pipe", "pipe"],
      env: { ...process.env, CODEX_DISABLE_UPDATE_CHECK: "1" },
    });
    let stdout = "";
    let stdoutBytes = 0;
    let stderr = "";
    let requestId = 1;
    let pageCount = 0;
    let settled = false;
    const models: DriverModel[] = [];
    const timer = setTimeout(() => fail(new Error("Codex model discovery timed out")), 15_000);
    let killTimer: ReturnType<typeof setTimeout> | undefined;

    const cleanup = () => {
      clearTimeout(timer);
      child.stdin.end();
      child.stdin.destroy();
      child.stdout.destroy();
      child.stderr.destroy();
      if (!child.killed) child.kill("SIGTERM");
      killTimer = setTimeout(() => {
        if (child.exitCode === null) child.kill("SIGKILL");
      }, 250);
      killTimer.unref?.();
    };
    const succeed = () => {
      if (settled) return;
      settled = true;
      cleanup();
      resolve(dedupeModels(models));
    };
    const fail = (error: Error) => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(error);
    };
    const send = (message: unknown) => {
      if (settled || child.stdin.destroyed || child.stdin.writableEnded) return;
      child.stdin.write(`${JSON.stringify(message)}\n`, (error) => {
        if (error) fail(error);
      });
    };
    const requestPage = (cursor: string | null = null) => {
      pageCount += 1;
      if (pageCount > maxPages) {
        fail(new Error("Codex model discovery returned too many pages"));
        return;
      }
      const id = requestId++;
      send({
        method: "model/list",
        id,
        params: { limit: 100, includeHidden: false, ...(cursor ? { cursor } : {}) },
      });
    };
    const consumeLine = (line: string) => {
      if (!line.trim()) return;
      let message: Record<string, unknown>;
      try {
        message = JSON.parse(line) as Record<string, unknown>;
      } catch {
        return;
      }
      if (message.id === 0) {
        if (message.error) return fail(new Error("Codex app-server initialization failed"));
        send({ method: "initialized", params: {} });
        requestPage();
        return;
      }
      if (typeof message.id === "number" && message.id > 0) {
        if (message.error) return fail(new Error("Codex model/list failed"));
        const page = parseCodexModelListResult(message.result);
        if (page.models.length > maxModels - models.length) {
          return fail(new Error("Codex model discovery returned too many models"));
        }
        models.push(...page.models);
        if (page.nextCursor) requestPage(page.nextCursor);
        else succeed();
      }
    };

    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      stdoutBytes += Buffer.byteLength(chunk, "utf8");
      if (stdoutBytes > maxOutputBytes) return fail(new Error("Codex model output was too large"));
      stdout += chunk;
      const lines = stdout.split(/\r?\n/);
      stdout = lines.pop() ?? "";
      lines.forEach(consumeLine);
    });
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => {
      stderr = `${stderr}${chunk}`.slice(-16_384);
    });
    child.stdin.on("error", (error) => fail(error));
    child.on("error", (error) => fail(error));
    child.on("close", () => {
      if (killTimer) clearTimeout(killTimer);
    });
    child.on("exit", () => {
      if (!settled) fail(new Error(stderr || "Codex app-server exited before model discovery"));
    });
    send({
      method: "initialize",
      id: 0,
      params: {
        clientInfo: { name: "choruz", title: "Choruz", version: "1.0.0" },
      },
    });
  });
}

export async function runModelCommand(
  binary: string,
  args: string[],
  timeoutMs = 15_000,
): Promise<CommandOutput> {
  return new Promise((resolve, reject) => {
    const maxOutputBytes = 8 * 1024 * 1024;
    const child = spawn(binary, args, { stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    let outputBytes = 0;
    let settled = false;
    let killTimer: ReturnType<typeof setTimeout> | undefined;
    const timeout = setTimeout(() => {
      finish(new Error("Model discovery timed out"));
    }, timeoutMs);

    const cleanup = () => {
      clearTimeout(timeout);
      child.stdout.destroy();
      child.stderr.destroy();
      if (!child.killed) child.kill("SIGTERM");
      killTimer = setTimeout(() => {
        if (child.exitCode === null) child.kill("SIGKILL");
      }, 250);
      killTimer.unref?.();
    };
    const finish = (error?: Error) => {
      if (settled) return;
      settled = true;
      cleanup();
      if (error) reject(error);
      else resolve({ stdout, stderr });
    };
    const append = (target: "stdout" | "stderr", chunk: Buffer) => {
      outputBytes += chunk.byteLength;
      if (outputBytes > maxOutputBytes) {
        finish(new Error("Model discovery output was too large"));
        return;
      }
      if (target === "stdout") stdout += chunk.toString("utf8");
      else stderr += chunk.toString("utf8");
    };

    child.stdout.on("data", (chunk: Buffer) => append("stdout", chunk));
    child.stderr.on("data", (chunk: Buffer) => append("stderr", chunk));
    child.on("error", (error) => finish(error));
    child.on("exit", (code, signal) => {
      if (code === 0) finish();
      else finish(new Error(stderr || `Model discovery exited with ${signal ?? code}`));
    });
    child.on("close", () => {
      if (killTimer) clearTimeout(killTimer);
    });
  });
}

function dedupeModels(models: DriverModel[]): DriverModel[] {
  return [...new Map(models.map((model) => [model.id, model])).values()];
}

function unavailable(driverId: DriverId, message: string): DriverModelDiscovery {
  return { driverId, status: "unavailable", models: [], message };
}

async function withTimeout<T>(promise: Promise<T>, ms: number, message: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error(message)), ms);
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}
