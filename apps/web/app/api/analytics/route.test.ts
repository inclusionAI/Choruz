import { NextRequest } from "next/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { requireAuth } from "../../../lib/api/api-auth";
import { POST } from "./route";

vi.mock("../../../lib/api/api-auth", () => ({
  requireAuth: vi.fn(),
}));

describe("/api/analytics redaction", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("redacts sensitive telemetry before logging the legacy analytics body", async () => {
    vi.mocked(requireAuth).mockResolvedValue({
      token: "session-token",
      claims: {
        principal_id: "user-a",
        workspace_id: "ws-a",
        display_name: "Alice",
        expires_at_epoch_s: 1,
      },
    });
    const logSpy = vi.spyOn(console, "log").mockImplementation(() => undefined);

    const response = await POST(
      new NextRequest("http://localhost/api/analytics", {
        method: "POST",
        body: JSON.stringify({
          event: "config_save",
          workspacePath: "/Users/alice/private/workspace",
          sessionToken: "session-token-test-value",
          attachment: {
            fileName: "private-plan.txt",
            attachmentBytes: "attachment-bytes-test-value",
          },
          message: {
            private: true,
            content: "private-content-test-value",
          },
        }),
        headers: { "content-type": "application/json" },
      }),
    );

    expect(response.status).toBe(200);
    const logged = JSON.stringify(logSpy.mock.calls);
    expect(logged).toContain("config_save");
    expect(logged).not.toContain("/Users/alice/private/workspace");
    expect(logged).not.toContain("session-token-test-value");
    expect(logged).not.toContain("private-plan.txt");
    expect(logged).not.toContain("attachment-bytes-test-value");
    expect(logged).not.toContain("private-content-test-value");
  });
});
