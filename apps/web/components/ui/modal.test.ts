import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { Modal } from "./modal";

it("never uses a dynamic title as the telemetry surface", () => {
  const html = renderToStaticMarkup(createElement(Modal, {
    title: "Customer confidential plan", onClose: () => {}, children: "Details",
  }));
  expect(html).toContain('data-activity-surface="dialog"');
  expect(html).not.toContain('data-activity-surface="Customer confidential plan"');
});

it("exposes an explicit static surface independently of the visible title", () => {
  const html = renderToStaticMarkup(createElement(Modal, {
    title: "Dynamic title", activitySurface: "shared-group-details", onClose: () => {}, children: "Details",
  }));
  expect(html).toContain('data-activity-surface="shared-group-details"');
});
