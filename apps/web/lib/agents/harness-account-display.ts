import type { AccountDriver, UsageWindow } from "./harness-accounts";

const CLAUDE_USAGE_LABELS: Record<string, string> = {
  five_hour: "5-hour",
  seven_day: "Weekly",
  seven_day_oauth_apps: "Weekly OAuth apps",
  seven_day_opus: "Weekly Opus",
  seven_day_sonnet: "Weekly Sonnet",
  extra_usage: "Monthly extra usage",
};

export function canonicalUsageLabel(driverType: AccountDriver, id: string, reportedLabel: string): string {
  return driverType === "claude_terminal" ? CLAUDE_USAGE_LABELS[id] ?? reportedLabel : reportedLabel;
}

/** Return quota windows in the canonical account-card order with stable product labels. */
export function displayUsageWindows(driverType: AccountDriver, windows: UsageWindow[]): UsageWindow[] {
  const order = (window: UsageWindow) => {
    if (driverType === "claude_terminal") {
      if (window.id === "five_hour") return 0;
      if (window.id === "seven_day") return 1;
    }
    if (driverType === "codex_terminal" && /^(codex|default):/.test(window.id)) return 0;
    return 2;
  };
  return windows
    .map((window) => ({ ...window, label: canonicalUsageLabel(driverType, window.id, window.label) }))
    .sort((left, right) => order(left) - order(right));
}
