import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { BrowserWorkflowDraft } from "./browser-workflow";

it("shows learned steps without a per-run authorization form", () => {
  const html = renderToStaticMarkup(createElement(BrowserWorkflowDraft, {
    workflow: { name: "Save draft", allowed_urls: ["https://example.org/editor"], steps: [
      { goal: "Enter title", operation: "type_text", labels: ['textbox "Title"'], value_key: "title" },
    ] },
  }));
  expect(html).toContain("Enter title");
  expect(html).not.toContain("<input");
  expect(html).not.toContain("<button");
  expect(html).toContain("matching new task");
});
