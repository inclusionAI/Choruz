import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { emptyOptimization, OptimizationFields } from "./experience-optimization";

it("starts with separate blank partitions and no application consent", () => {
  const settings = emptyOptimization();
  expect(settings.auto_apply).toBe(false);
  expect(settings.config.evolve_team).toBe(false);
  expect(settings.suite.cases.map((row) => row.split)).toEqual(["train", "validation", "test"]);
  expect(settings.suite.cases.every((row) => row.input === "")).toBe(true);
  settings.suite.cases[0].input = "One owner's task";
  expect(emptyOptimization().suite.cases[0].input).toBe("");
});

it("shows the configured per-run cost and disables edits while saving", () => {
  const settings = emptyOptimization();
  settings.config.max_metric_calls = 48;
  settings.config.max_proposals = 5;
  const html = renderToStaticMarkup(createElement(OptimizationFields, { value: settings, onChange: () => {}, disabled: true }));
  expect(html).toContain('<fieldset disabled=""');
  expect(html).toContain("at most 192 execution calls including collaborators");
  expect(html).toContain("5 proposal calls and one final review");
  expect(html).toContain("not a token or billing limit");
});
