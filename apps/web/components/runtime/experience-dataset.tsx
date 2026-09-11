export type DatasetReport = {
  version: string; previous_version: string; total: number; added: number; updated: number;
  withdrawn: number; unevaluable: number; duplicate_groups: number; conflicting_groups: number;
  categories: Record<string, number>; outcomes: Record<string, number>;
  variants?: number;
};

export function ExperienceDataset({ report }: { report: DatasetReport }) {
  return <section aria-label="Dataset quality report" className="modal-form">
    <h4>Dataset quality</h4>
    <p title={report.version}>Version {report.version.slice(0, 12)} · {report.total} objectives</p>
    <p>{report.added} added · {report.updated} updated · {report.withdrawn} withdrawn</p>
    <p>{report.duplicate_groups} duplicate groups · {report.conflicting_groups} conflicting groups · {report.unevaluable} unevaluable</p>
    <p>{report.variants ?? 0} reviewed training paraphrases · one vote per objective</p>
    <dl>{Object.entries(report.categories).map(([category, count]) => <div key={category}><dt>{category}</dt><dd>{count}</dd></div>)}</dl>
    <p>{Object.entries(report.outcomes).map(([outcome, count]) => `${outcome}: ${count}`).join(" · ")}</p>
    <p className="field-hint">Coverage describes the bounded working corpus, not all historical work. Difficult and incomplete objectives stay recorded. Conflicting groups are excluded from scoring. Original evidence is revisited daily while learning is enabled.</p>
  </section>;
}

export type TaskPerformance = { context: string; tasks: { episode_ref: string; samples: number; successes: number; band: string }[] };

export function ExperiencePerformance({ report }: { report: TaskPerformance }) {
  return <section aria-label="Measured task difficulty" className="modal-form">
    <h4>Measured task difficulty</h4>
    <p title={report.context}>Current executor context {report.context.slice(0, 12)} · current active guidance</p>
    {report.tasks.length === 0 ? <p>No matching completed training trials yet.</p> : <ul>{report.tasks.map(task => <li key={task.episode_ref}>{task.episode_ref}: {task.successes}/{task.samples} correct · {task.band}</li>)}</ul>}
    <p className="field-hint">At least 3 completed baseline runs are needed for a band: easy ≥80%, hard ≤20%, otherwise mixed. These are observed rates, not intrinsic difficulty. Errors, inconclusive answers, changed tasks and held-out scores are excluded. History is limited to the latest 100 matching runs.</p>
  </section>;
}
