"use client";

import type { SetupInput } from "../../lib/groups/team-templates";
import { PathPicker } from "../workspace/path-picker";

/**
 * One template setup input. Renders the control for each `SetupInputType`
 * (text, textarea, select, path) plus the description / required hint, so
 * every template-driven form shows the same field for the same input.
 */
export function SetupInputField({
  input,
  value,
  onChange,
}: {
  input: SetupInput;
  value: string;
  onChange: (value: string) => void;
}) {
  let control: React.ReactNode;
  switch (input.type) {
    case "textarea":
      control = (
        <textarea
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={input.placeholder}
          rows={3}
        />
      );
      break;
    case "select":
      control = (
        <select value={value} onChange={(e) => onChange(e.target.value)}>
          <option value="">Select…</option>
          {input.options?.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      );
      break;
    case "path":
      // Template metadata is typed or browsed to, never pre-filled with $HOME:
      // an empty repository path is omitted from the agent's instructions.
      control = (
        <PathPicker value={value} onChange={onChange} placeholder={input.placeholder} autoHome={false} />
      );
      break;
    default:
      control = (
        <input
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={input.placeholder}
        />
      );
  }
  const hint = [input.description, input.required ? "Required." : ""].filter(Boolean).join(" ");
  // Layout (stacked label, gap) comes from the enclosing .modal-form.
  return (
    <label>
      {input.label}
      {control}
      {hint && <span className="field-hint">{hint}</span>}
    </label>
  );
}
