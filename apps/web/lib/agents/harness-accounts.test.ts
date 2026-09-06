import { describe, expect, it } from "vitest";

import { harnessAccountEnv, type HarnessAccount } from "./harness-accounts";
import { displayUsageWindows } from "./harness-account-display";

const baseAccount: Pick<HarnessAccount, "id" | "driverType" | "profileKind"> = {
  id: "12345678-1234-1234-1234-123456789abc",
  driverType: "claude_terminal",
  profileKind: "isolated",
};

describe("harness account profiles", () => {
  it("maps isolated profiles to a single harness-specific env var", () => {
    const env = harnessAccountEnv(baseAccount);
    expect(Object.keys(env)).toEqual(["CLAUDE_CONFIG_DIR"]);
    expect(env.CLAUDE_CONFIG_DIR).toMatch(/12345678-1234-1234-1234-123456789abc\/claude$/);
    expect(harnessAccountEnv({ ...baseAccount, driverType: "codex_terminal" })).toEqual({
      CODEX_HOME: expect.stringMatching(/12345678-1234-1234-1234-123456789abc\/codex$/),
    });
    expect(harnessAccountEnv({ ...baseAccount, profileKind: "default" })).toEqual({});
  });

  it("normalizes persisted Claude window labels from every snapshot producer", () => {
    expect(displayUsageWindows("claude_terminal", [
      { id: "nimbus_quill", label: "nimbus quill", usedPercent: 0, remainingPercent: 100, resetsAt: null, windowDurationMinutes: null },
      { id: "seven_day", label: "seven day", usedPercent: 39, remainingPercent: 61, resetsAt: null, windowDurationMinutes: null },
      { id: "five_hour", label: "five hour", usedPercent: 20, remainingPercent: 80, resetsAt: null, windowDurationMinutes: null },
    ])).toEqual([
      expect.objectContaining({ id: "five_hour", label: "5-hour" }),
      expect.objectContaining({ id: "seven_day", label: "Weekly" }),
      expect.objectContaining({ id: "nimbus_quill", label: "nimbus quill" }),
    ]);
  });

  it("puts the default Codex quota before named product windows", () => {
    const window = (id: string, label: string) => ({ id, label, usedPercent: 0, remainingPercent: 100, resetsAt: null, windowDurationMinutes: null });
    expect(displayUsageWindows("codex_terminal", [
      window("codex_bengalfox:primary", "GPT-5.3-Codex-Spark 5-hour"),
      window("codex_bengalfox:secondary", "GPT-5.3-Codex-Spark Weekly"),
      window("codex:primary", "Weekly"),
    ]).map(({ id }) => id)).toEqual([
      "codex:primary",
      "codex_bengalfox:primary",
      "codex_bengalfox:secondary",
    ]);
  });
});
