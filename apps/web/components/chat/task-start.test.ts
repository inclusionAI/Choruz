import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, expect, it, vi } from "vitest";
import { useDriverAvailability } from "../../hooks/use-driver-availability";
import { TaskStart } from "./task-start";

vi.mock("../../hooks/use-driver-availability", () => ({ useDriverAvailability: vi.fn() }));
afterEach(() => vi.resetAllMocks());

it("offers an installed Muse CLI when no other default CLI is installed", () => {
  vi.mocked(useDriverAvailability).mockReturnValue({
    availability: [{ driverId: "muse_terminal", label: "Muse Code", status: "available", available: true, reason: "Installed", setupHint: "" }],
    loaded: true,
    error: null,
  });
  const html = renderToStaticMarkup(createElement(TaskStart, {
    principalId: "operator", companyId: "project", projectName: "Project",
    onCreated: async () => {}, onAdvanced: () => {},
  }));
  expect(html).toContain('<option value="muse_terminal" selected="">Muse Code</option>');
  expect(html).not.toContain("No supported CLI installed");
});
