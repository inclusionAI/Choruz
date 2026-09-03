import { describe, expect, it } from "vitest";

import { sanitizeTelemetryData, sanitizeTelemetryValue } from "./telemetry-sanitize";

describe("telemetry sanitization", () => {
  it("redacts camelCase path, token, private content, filename, and byte fields", () => {
    const sanitized = sanitizeTelemetryData({
      conversation_id: "conv-safe",
      workspacePath: "/Users/alice/private/workspace",
      filePath: "/tmp/private.txt",
      sessionToken: "session-token-test-value",
      attachment: {
        fileName: "private-plan.txt",
        attachmentBytes: "attachment-bytes-test-value",
      },
      message: {
        private: true,
        content: "private-content-test-value",
        preview: "private-preview-test-value",
      },
    });

    const serialized = JSON.stringify(sanitized);
    expect(serialized).not.toContain("/Users/alice/private/workspace");
    expect(serialized).not.toContain("/tmp/private.txt");
    expect(serialized).not.toContain("session-token-test-value");
    expect(serialized).not.toContain("private-plan.txt");
    expect(serialized).not.toContain("attachment-bytes-test-value");
    expect(serialized).not.toContain("private-content-test-value");
    expect(serialized).not.toContain("private-preview-test-value");
    expect(sanitized).toMatchObject({
      conversation_id: "conv-safe",
      workspacePath: "[REDACTED]",
      filePath: "[REDACTED]",
      sessionToken: "[REDACTED]",
      attachment: {
        fileName: "[REDACTED]",
        attachmentBytes: "[REDACTED]",
      },
      message: {
        content: "[REDACTED]",
        preview: "[REDACTED]",
      },
    });
  });

  it("redacts marker-shaped secrets from non-sensitive string fields", () => {
    const sanitized = sanitizeTelemetryData({
      reason: "request failed: Bearer real-token-value",
      error: "token=token-test-value secret:secret-test-value password=password-test-value",
      nested: {
        detail: "bearer lower-token token: colon-token secret=equals-secret password: colon-password",
      },
    });

    const serialized = JSON.stringify(sanitized);
    for (const leaked of [
      "real-token-value",
      "token-test-value",
      "secret-test-value",
      "password-test-value",
      "lower-token",
      "colon-token",
      "equals-secret",
      "colon-password",
    ]) {
      expect(serialized).not.toContain(leaked);
    }
    expect(sanitized).toMatchObject({
      reason: "request failed: Bearer [REDACTED]",
      error: "token=[REDACTED] secret:[REDACTED] password=[REDACTED]",
      nested: {
        detail: "bearer [REDACTED] token:[REDACTED] secret=[REDACTED] password:[REDACTED]",
      },
    });
  });

  it("preserves non-sensitive correlation fields", () => {
    expect(
      sanitizeTelemetryValue({ conversation_id: "conv-1", content_len: 42, driverType: "codex" }),
    ).toEqual({ conversation_id: "conv-1", content_len: 42, driverType: "codex" });
  });
});
