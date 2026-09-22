export type BrowserWorkflow = {
  name: string;
  allowed_urls: string[];
  steps: { goal: string; operation: "click" | "type_text" | "select"; labels: string[]; value_key?: string | null }[];
};

export function BrowserWorkflowDraft({ workflow }: { workflow: BrowserWorkflow }) {
  return <section aria-label="Learned browser workflow">
    <h4>{workflow.name}</h4>
    <p className="field-hint">With browser automation enabled, your Agent can use this workflow for a matching new task. The first execution checks the actual outcome; historical tasks are not replayed.</p>
    <ol>{workflow.steps.map((step, index) => <li key={index}>{step.goal}</li>)}</ol>
  </section>;
}
