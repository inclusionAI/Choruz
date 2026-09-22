import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { DecisionHistory, ExperienceDecisions } from "./experience-decisions";

it("does not grant provider consent or show model settings by default", () => {
  const html = renderToStaticMarkup(createElement(ExperienceDecisions, { endpoint: "/test", sessionToken: "fixture", initial: null, bindings: [], onSaved: () => {} }));
  expect(html).toContain("Allow this Agent’s task evidence to be sent to TypeSafe");
  expect(html).not.toContain("checked=");
  expect(html).not.toContain("Exact provider model ID");
});

it("distinguishes advisory failure and test success from activation", () => {
  const html = renderToStaticMarkup(createElement(DecisionHistory, { evidence: {
    decisions: { supervise: { outcome: "failed", error_category: "validation" } },
    program_trial: { status: "validated", results: [{ score: 1, split: "validation" }, { score: 1, split: "test" }] },
  } }));
  expect(html).toContain("advisory");
  expect(html).toContain("failed");
  expect(html).toContain("not active");
  expect(html).toContain("2 / 2 evaluated cases passed");
});
