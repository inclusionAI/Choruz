export const DRIVER_IDS = [
  "claude_terminal",
  "codex_terminal",
  "muse_terminal",
  "pi_terminal",
  "grok_terminal",
  "opencode_terminal",
  "mathcode_terminal",
  "codex_exec",
  "webhook_agent",
] as const;

export type DriverId = (typeof DRIVER_IDS)[number];

export type DriverBinaryEnvVar =
  | "CHORUZ_CLAUDE_BINARY"
  | "CHORUZ_CODEX_BINARY"
  | "CHORUZ_MUSE_BINARY"
  | "CHORUZ_PI_BINARY"
  | "CHORUZ_GROK_BINARY"
  | "CHORUZ_OPENCODE_BINARY"
  | "CHORUZ_MATHCODE_BINARY";

type DriverBinaryDefinition = {
  envVar: DriverBinaryEnvVar;
  runtimeEnvVar?: string;
  defaultBinary: string;
};

const CODEX_BINARY: DriverBinaryDefinition = {
  envVar: "CHORUZ_CODEX_BINARY", runtimeEnvVar: "CHORUZ_CODEX_CLI_PATH", defaultBinary: "codex",
};

export const DRIVER_BINARIES: Record<DriverId | "codex_app_server", DriverBinaryDefinition | undefined> = {
  claude_terminal: { envVar: "CHORUZ_CLAUDE_BINARY", runtimeEnvVar: "CHORUZ_CLAUDE_CLI_PATH", defaultBinary: "claude" },
  codex_terminal: CODEX_BINARY,
  muse_terminal: { envVar: "CHORUZ_MUSE_BINARY", runtimeEnvVar: "CHORUZ_MUSE_CLI_PATH", defaultBinary: "muse" },
  codex_exec: CODEX_BINARY,
  codex_app_server: CODEX_BINARY,
  pi_terminal: { envVar: "CHORUZ_PI_BINARY", runtimeEnvVar: "CHORUZ_PI_CLI_PATH", defaultBinary: "pi" },
  grok_terminal: { envVar: "CHORUZ_GROK_BINARY", runtimeEnvVar: "CHORUZ_GROK_CLI_PATH", defaultBinary: "grok" },
  opencode_terminal: { envVar: "CHORUZ_OPENCODE_BINARY", runtimeEnvVar: "CHORUZ_OPENCODE_CLI_PATH", defaultBinary: "opencode" },
  mathcode_terminal: { envVar: "CHORUZ_MATHCODE_BINARY", defaultBinary: "mathcode" },
  webhook_agent: undefined,
};

/** Resolve within the supplied environment; webhook agents have no executable. */
export function resolveDriverBinary(
  driverId: DriverId | "codex_app_server",
  env: Record<string, string | undefined> = process.env,
): string | undefined {
  const definition = DRIVER_BINARIES[driverId];
  if (!definition) return undefined;
  return env[definition.envVar]?.trim()
    || (definition.runtimeEnvVar ? env[definition.runtimeEnvVar]?.trim() : undefined)
    || definition.defaultBinary;
}

export const LOCAL_TERMINAL_DRIVER_IDS: DriverId[] = [
  "claude_terminal",
  "codex_terminal",
  "muse_terminal",
  "pi_terminal",
  "grok_terminal",
  "opencode_terminal",
];

export const DRIVER_PLUGIN_IDS: Partial<Record<DriverId, string>> = {
  grok_terminal: "grok",
  pi_terminal: "pi",
  opencode_terminal: "opencode",
  mathcode_terminal: "mathcode",
};

export function enabledDriverIds(drivers: readonly DriverId[], pluginIds: ReadonlySet<string>): DriverId[] {
  return drivers.filter((driver) => {
    const plugin = DRIVER_PLUGIN_IDS[driver];
    return !plugin || pluginIds.has(plugin);
  });
}

/** Plugin gating affects new agents, not the identity of existing terminal sessions. */
export function creatableAgentDriverIds(pluginIds: ReadonlySet<string>): readonly DriverId[] {
  const drivers: DriverId[] = [...LOCAL_TERMINAL_DRIVER_IDS, "mathcode_terminal", "webhook_agent"];
  return enabledDriverIds(drivers, pluginIds);
}

const LOCAL_TERMINAL_DRIVER_ID_SET = new Set<string>(LOCAL_TERMINAL_DRIVER_IDS);

export function isTerminalDriver(driverType: string): boolean {
  return LOCAL_TERMINAL_DRIVER_ID_SET.has(driverType);
}

type DriverDefinition = {
  label: string;
  driverId: DriverId;
  setupHint: string;
};

export const DRIVER_DEFINITIONS: DriverDefinition[] = [
  {
    label: "Muse Code",
    driverId: "muse_terminal",
    setupHint: "Install Muse Code or set CHORUZ_MUSE_BINARY to an executable path.",
  },
  {
    label: "Claude Code",
    driverId: "claude_terminal",
    setupHint: "Install the Claude CLI or set CHORUZ_CLAUDE_BINARY to an executable path.",
  },
  {
    label: "Codex",
    driverId: "codex_terminal",
    setupHint: "Install the Codex CLI or set CHORUZ_CODEX_BINARY to an executable path.",
  },
  {
    label: "Codex",
    driverId: "codex_exec",
    setupHint: "Install the Codex CLI or set CHORUZ_CODEX_BINARY to an executable path.",
  },
  {
    label: "Pi Agent",
    driverId: "pi_terminal",
    setupHint: "Install Pi Agent or set CHORUZ_PI_BINARY to an executable path.",
  },
  {
    label: "Grok Build",
    driverId: "grok_terminal",
    setupHint: "Install Grok Build or set CHORUZ_GROK_BINARY to an executable path.",
  },
  {
    label: "OpenCode",
    driverId: "opencode_terminal",
    setupHint: "Install OpenCode or set CHORUZ_OPENCODE_BINARY to an executable path.",
  },
  {
    label: "MathCode",
    driverId: "mathcode_terminal",
    setupHint: "Install MathCode or set CHORUZ_MATHCODE_BINARY to an executable path.",
  },
  {
    label: "External agent",
    driverId: "webhook_agent",
    setupHint: "Provide an HTTPS webhook endpoint when creating the agent.",
  },
];

const DRIVER_LABELS = new Map(DRIVER_DEFINITIONS.map(({ driverId, label }) => [driverId, label]));

/** Human-readable driver name; unknown ids fall back to the raw id. */
export function driverDisplayName(driverType: string): string {
  return DRIVER_LABELS.get(driverType as DriverId) ?? driverType;
}
