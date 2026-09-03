import { constants as fsConstants, promises as fs } from "node:fs";
import { execFile } from "node:child_process";
import path from "node:path";
import { promisify } from "node:util";

import type { DriverId } from "../groups/team-templates";

export type DriverAvailabilityStatus = "available" | "unavailable";
export type DriverBinaryEnvVar =
  | "CHORUZ_CLAUDE_BINARY"
  | "CHORUZ_CODEX_BINARY"
  | "CHORUZ_PI_BINARY"
  | "CHORUZ_GROK_BINARY"
  | "CHORUZ_OPENCODE_BINARY"
  | "CHORUZ_MATHCODE_BINARY";

export type DriverAvailabilityItem = {
  label: string;
  driverId: DriverId;
  status: DriverAvailabilityStatus;
  available: boolean;
  reason: string;
  setupHint: string;
  envVar?: DriverBinaryEnvVar;
};

export type DriverBinaryCheck = (binaryPath: string) => boolean | Promise<boolean>;
export type DriverAvailabilityEnv = Record<string, string | undefined>;
export type PiRuntimeCheckResult = {
  available: boolean;
  reason?: string;
  setupHint?: string;
};
export type PiRuntimeCheck = (
  binaryPath: string,
  env: DriverAvailabilityEnv,
) => PiRuntimeCheckResult | Promise<PiRuntimeCheckResult>;

const execFileAsync = promisify(execFile);

type DriverDefinition = {
  label: string;
  driverId: DriverId;
  envVar?: DriverBinaryEnvVar;
  runtimeEnvVar?: string;
  defaultBinary?: string;
  setupHint: string;
};

const DRIVER_DEFINITIONS: DriverDefinition[] = [
  {
    label: "Claude",
    driverId: "claude_terminal",
    envVar: "CHORUZ_CLAUDE_BINARY",
    runtimeEnvVar: "CHORUZ_CLAUDE_CLI_PATH",
    defaultBinary: "claude",
    setupHint: "Install the Claude CLI or set CHORUZ_CLAUDE_BINARY to an executable path.",
  },
  {
    label: "Codex Terminal",
    driverId: "codex_terminal",
    envVar: "CHORUZ_CODEX_BINARY",
    runtimeEnvVar: "CHORUZ_CODEX_CLI_PATH",
    defaultBinary: "codex",
    setupHint: "Install the Codex CLI or set CHORUZ_CODEX_BINARY to an executable path.",
  },
  {
    label: "Codex (headless)",
    driverId: "codex_exec",
    envVar: "CHORUZ_CODEX_BINARY",
    runtimeEnvVar: "CHORUZ_CODEX_CLI_PATH",
    defaultBinary: "codex",
    setupHint: "Install the Codex CLI or set CHORUZ_CODEX_BINARY to an executable path.",
  },
  {
    label: "Pi Agent",
    driverId: "pi_terminal",
    envVar: "CHORUZ_PI_BINARY",
    runtimeEnvVar: "CHORUZ_PI_CLI_PATH",
    defaultBinary: "pi",
    setupHint: "Install Pi Agent or set CHORUZ_PI_BINARY to an executable path.",
  },
  {
    label: "Grok Build",
    driverId: "grok_terminal",
    envVar: "CHORUZ_GROK_BINARY",
    runtimeEnvVar: "CHORUZ_GROK_CLI_PATH",
    defaultBinary: "grok",
    setupHint: "Install Grok Build or set CHORUZ_GROK_BINARY to an executable path.",
  },
  {
    label: "OpenCode",
    driverId: "opencode_terminal",
    envVar: "CHORUZ_OPENCODE_BINARY",
    runtimeEnvVar: "CHORUZ_OPENCODE_CLI_PATH",
    defaultBinary: "opencode",
    setupHint: "Install OpenCode or set CHORUZ_OPENCODE_BINARY to an executable path.",
  },
  {
    label: "MathCode",
    driverId: "mathcode_terminal",
    envVar: "CHORUZ_MATHCODE_BINARY",
    defaultBinary: "mathcode",
    setupHint: "Install MathCode or set CHORUZ_MATHCODE_BINARY to an executable path.",
  },
  {
    label: "Webhook",
    driverId: "webhook_agent",
    setupHint: "Provide an HTTPS webhook endpoint when creating the agent.",
  },
];

export function resolveDriverBinary(driverId: DriverId, env: DriverAvailabilityEnv = process.env): string | undefined {
  const definition = DRIVER_DEFINITIONS.find((item) => item.driverId === driverId);
  if (!definition?.envVar) return undefined;
  return env[definition.envVar]?.trim()
    || (definition.runtimeEnvVar ? env[definition.runtimeEnvVar]?.trim() : undefined)
    || definition.defaultBinary;
}

export async function getDriverAvailability(options: {
  env?: DriverAvailabilityEnv;
  checkBinary?: DriverBinaryCheck;
  checkPiRuntime?: PiRuntimeCheck;
} = {}): Promise<DriverAvailabilityItem[]> {
  const env = options.env ?? process.env;
  const checkBinary = options.checkBinary
    ?? ((binaryPath: string) => defaultBinaryCheck(binaryPath, env));
  const checkPiRuntime = options.checkPiRuntime ?? defaultPiRuntimeCheck;

  return Promise.all(
    DRIVER_DEFINITIONS.map(async (definition) => {
      if (definition.driverId === "webhook_agent") {
        return {
          label: definition.label,
          driverId: definition.driverId,
          status: "available",
          available: true,
          reason: "Webhook agents do not require a local CLI.",
          setupHint: definition.setupHint,
        } satisfies DriverAvailabilityItem;
      }

      const binaryPath = resolveDriverBinary(definition.driverId, env) ?? definition.defaultBinary ?? "";
      let available = binaryPath ? await checkBinary(binaryPath) : false;
      let runtimeReason: string | undefined;
      let runtimeSetupHint: string | undefined;
      if (available && definition.driverId === "pi_terminal") {
        const runtime = await checkPiRuntime(binaryPath, env);
        available = runtime.available;
        runtimeReason = runtime.reason;
        runtimeSetupHint = runtime.setupHint;
      }
      return {
        label: definition.label,
        driverId: definition.driverId,
        status: available ? "available" : "unavailable",
        available,
        reason: runtimeReason ?? (available
          ? `${definition.label} CLI is available.`
          : `${definition.label} CLI was not found.`),
        setupHint: runtimeSetupHint ?? definition.setupHint,
        envVar: definition.envVar,
      } satisfies DriverAvailabilityItem;
    }),
  );
}

export async function defaultPiRuntimeCheck(
  binaryPath: string,
  env: DriverAvailabilityEnv = process.env,
): Promise<PiRuntimeCheckResult> {
  const executable = await resolveExecutable(binaryPath, env);
  if (!executable) {
    return {
      available: false,
      reason: "Pi Agent executable could not be resolved for runtime validation.",
      setupHint: "Install Pi Agent or set CHORUZ_PI_BINARY to an executable path.",
    };
  }

  let entrypoint: string;
  let shebang: string;
  try {
    entrypoint = await fs.realpath(/* turbopackIgnore: true */ executable);
    shebang = (await fs.readFile(/* turbopackIgnore: true */ entrypoint, "utf8"))
      .split(/\r?\n/, 1)[0] ?? "";
  } catch {
    return {
      available: false,
      reason: "Pi Agent executable could not be inspected for Node compatibility.",
      setupHint: "Reinstall Pi Agent or set CHORUZ_PI_BINARY to a valid executable wrapper.",
    };
  }

  const nodeBinary = await nodeInterpreterFromShebang(shebang, env);
  // An explicit native/shell wrapper may manage its own compatible runtime.
  // Do not guess a different global Node binary behind the operator's wrapper.
  if (!nodeBinary) return { available: true };

  try {
    const { stdout } = await execFileAsync(
      nodeBinary,
      [
        "-e",
        "const z=require('node:zlib');process.stdout.write(JSON.stringify({version:process.version,zstd:typeof z.createZstdDecompress==='function'}))",
      ],
      {
        env: { ...process.env, ...env },
        timeout: 5_000,
        maxBuffer: 16 * 1024,
      },
    );
    const probe = JSON.parse(stdout) as { version?: string; zstd?: boolean };
    if (probe.zstd) return { available: true };

    return incompatiblePiNode(probe.version, "a Node runtime with zlib.createZstdDecompress");
  } catch {
    return {
      available: false,
      reason: "Pi Agent's configured Node runtime could not complete its compatibility probe.",
      setupHint:
        "Put a Pi-compatible Node runtime first on PATH, or set CHORUZ_PI_BINARY to an explicit wrapper that launches Pi with one.",
    };
  }
}

export async function defaultBinaryCheck(
  binaryPath: string,
  env: DriverAvailabilityEnv = process.env,
): Promise<boolean> {
  if (binaryPath.includes("/") || binaryPath.startsWith(".")) {
    return canExecute(binaryPath);
  }

  const pathEntries = (env.PATH ?? "")
    .split(path.delimiter)
    .filter(Boolean);

  for (const entry of pathEntries) {
    if (await canExecute(path.join(entry, binaryPath))) return true;
  }
  return false;
}

async function canExecute(candidate: string): Promise<boolean> {
  try {
    await fs.access(candidate, fsConstants.X_OK);
    return true;
  } catch {
    return false;
  }
}

async function resolveExecutable(
  binaryPath: string,
  env: DriverAvailabilityEnv,
): Promise<string | undefined> {
  if (binaryPath.includes("/") || binaryPath.startsWith(".")) {
    return (await canExecute(binaryPath)) ? path.resolve(binaryPath) : undefined;
  }
  for (const entry of (env.PATH ?? process.env.PATH ?? "").split(path.delimiter).filter(Boolean)) {
    const candidate = path.join(entry, binaryPath);
    if (await canExecute(candidate)) return candidate;
  }
  return undefined;
}

async function nodeInterpreterFromShebang(
  shebang: string,
  env: DriverAvailabilityEnv,
): Promise<string | undefined> {
  if (/^#!\s*\/usr\/bin\/env(?:\s+-S)?\s+node(?:\s|$)/.test(shebang)) {
    return resolveExecutable("node", env);
  }
  const match = shebang.match(/^#!\s*(\/\S*\/node)(?:\s|$)/);
  return match?.[1];
}

function incompatiblePiNode(version: string | undefined, requirement: string): PiRuntimeCheckResult {
  const actual = version?.trim() || "an unknown Node version";
  return {
    available: false,
    reason: `Pi Agent requires ${requirement}; its configured launcher resolves ${actual}, which is incompatible.`,
    setupHint:
      "Put a compatible Node runtime first on PATH, or set CHORUZ_PI_BINARY to an explicit wrapper that launches Pi with one. Choruz will not guess another global Node installation.",
  };
}
