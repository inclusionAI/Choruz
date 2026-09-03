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

export const LOCAL_TERMINAL_DRIVER_IDS: DriverId[] = [
  "claude_terminal",
  "codex_terminal",
  "pi_terminal",
  "grok_terminal",
  "opencode_terminal",
];

/** Drivers a user can pick when creating a single agent. */
export const CREATABLE_AGENT_DRIVER_IDS: readonly DriverId[] = [
  ...LOCAL_TERMINAL_DRIVER_IDS,
  "webhook_agent",
];

/** MathCode is supplied by the opt-in mathcode plugin, not the core driver set. */
export function creatableAgentDriverIds(mathcodeEnabled: boolean): readonly DriverId[] {
  return mathcodeEnabled
    ? [...LOCAL_TERMINAL_DRIVER_IDS, "mathcode_terminal", "webhook_agent"]
    : CREATABLE_AGENT_DRIVER_IDS;
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
