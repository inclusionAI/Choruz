/** Read-only step indicator for multi-step dialogs ("Setup · Review · …"). */
export function StepTabs({
  steps,
  active,
  label,
}: {
  steps: ReadonlyArray<{ id: string; label: string }>;
  active: string;
  label: string;
}) {
  return (
    <div className="step-tabs" aria-label={label}>
      {steps.map((step) => (
        <span
          key={step.id}
          className={`step-tab${step.id === active ? " active" : ""}`}
          aria-current={step.id === active ? "step" : undefined}
        >
          {step.label}
        </span>
      ))}
    </div>
  );
}
