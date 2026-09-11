import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { ExperienceDataset, ExperiencePerformance } from "./experience-dataset";

it("distinguishes withdrawals, conflicts and coverage without calling incomplete work an error", () => {
  const html = renderToStaticMarkup(createElement(ExperienceDataset, { report: {
    version: "abcdef123456789", previous_version: "older", total: 9, added: 2, updated: 3,
    withdrawn: 1, unevaluable: 4, duplicate_groups: 2, conflicting_groups: 1,
    categories: { research: 6, math: 3 }, outcomes: { incomplete: 4, retried: 5 },
  } }));
  for (const text of ["Version abcdef123456", "9 objectives", "1 withdrawn", "1 conflicting groups", "research", "incomplete: 4", "retried: 5", "not all historical work"]) expect(html).toContain(text);
});

it("shows measured evidence and the absence of matching trials without inventing a difficulty", () => {
  const report={context:"abcdef123456789",tasks:[{episode_ref:"task-a",samples:2,successes:0,band:"insufficient"}]};
  const html=renderToStaticMarkup(createElement(ExperiencePerformance,{report}));
  for(const text of ["task-a: 0/2 correct", "insufficient", "current active guidance", "not intrinsic difficulty", "held-out scores are excluded"]) expect(html).toContain(text);
  expect(renderToStaticMarkup(createElement(ExperiencePerformance,{report:{...report,tasks:[]}}))).toContain("No matching completed training trials yet");
});
