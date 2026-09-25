export const DRIVER_IDS = [
  "claude_terminal",
  "codex_terminal",
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
  "pi_terminal",
  "grok_terminal",
  "opencode_terminal",
];

export const DRIVER_PLUGIN_IDS: Partial<Record<DriverId, string>> = {
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

const DRIVER_LABELS: Record<DriverId, string> = {
  claude_terminal: "Claude Code",
  codex_terminal: "Codex",
  codex_exec: "Codex",
  pi_terminal: "Pi Agent",
  grok_terminal: "Grok Build",
  opencode_terminal: "OpenCode",
  mathcode_terminal: "MathCode",
  webhook_agent: "External agent",
};

/** Human-readable driver name; unknown ids fall back to the raw id. */
export function driverDisplayName(driverType: string): string {
  return DRIVER_LABELS[driverType as DriverId] ?? driverType;
}
