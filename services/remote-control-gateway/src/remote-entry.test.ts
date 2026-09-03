import { describe, expect, it } from "vitest";

import { remoteEntryResponse } from "./remote-entry";

describe("remoteEntryResponse", () => {
  it("redirects to the hosted dashboard's Remote page with the gateway origin", () => {
    const response = remoteEntryResponse(
      new URL("https://gateway.example/remote?device_name=Phone"),
      "https://app.example/base/",
    );
    expect(response.status).toBe(302);
    expect(response.headers.get("location")).toBe(
      "https://app.example/remote?device_name=Phone&gateway=https%3A%2F%2Fgateway.example",
    );
  });

  it("explains where the Remote page lives when no dashboard is hosted", async () => {
    const response = remoteEntryResponse(new URL("https://gateway.example/"), undefined);
    expect(response.status).toBe(404);
    expect(response.headers.get("content-type")).toContain("text/plain");
    await expect(response.text()).resolves.toContain("/remote?gateway=https://gateway.example");
  });
});
