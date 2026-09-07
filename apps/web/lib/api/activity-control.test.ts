import { describe, expect, it } from "vitest";
import { activityValue } from "./activity-control";

describe("component value snapshots", () => {
  it("retains ordinary edits, selections and clearing", () => {
    expect(activityValue("review plan", "Message")).toEqual({ value: "review plan", value_length: 11, value_truncated: false });
    expect(activityValue("", "Message")).toEqual({ value: "", value_length: 0, value_truncated: false });
    expect(activityValue("host-b", "Account device").value).toBe("host-b");
  });
  it.each(["password", "Claude Code authentication value", "Codex callback URL", "Invitation to join", "API key", "Pairing credential", "one-time-code"])("excludes %s values and lengths", identity => {
    expect(activityValue("never-store-this", identity)).toEqual({ value_omitted: "private" });
  });
  it("honors explicit private fields and filesystem policy", () => {
    expect(activityValue("secret", "generic", true)).toEqual({ value_omitted: "private" });
    expect(activityValue("/home/person", "Workspace folder")).toEqual({ value_omitted: "filesystem" });
  });
  it("redacts recognizable secrets before bounding the snapshot", () => {
    expect(activityValue("use Bearer abc123", "Message").value).toBe("use Bearer [REDACTED]");
    expect(activityValue("x".repeat(3000), "Message")).toEqual({ value: "x".repeat(2048), value_length: 3000, value_truncated: true });
  });
});
