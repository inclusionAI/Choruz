"use client";

import type { AgentInstructionFields } from "../../lib/agents/agent-instructions";
import { INSTRUCTION_FIELDS } from "../../lib/agents/agent-instructions";

type Props = {
  fields: AgentInstructionFields;
  onChange: (fields: AgentInstructionFields) => void;
};

export function AgentInstructionForm({ fields, onChange }: Props) {
  return (
    <div className="instruction-form">
      {INSTRUCTION_FIELDS.map((meta) => {
        const textareaId = `instruction-field-${meta.key}`;
        const helpId = `${textareaId}-help`;
        return (
          <div key={meta.key} className="instruction-field">
            <div className="instruction-field-heading">
              <label className="instruction-field-label" htmlFor={textareaId}>{meta.label}</label>
              <span
                className="instruction-field-help"
                role="img"
                tabIndex={0}
                aria-label={`About ${meta.label}`}
                aria-describedby={helpId}
                data-help={meta.help}
              >
                i
              </span>
              <span id={helpId} hidden>{meta.help}</span>
            </div>
            <textarea
              id={textareaId}
              className="instruction-field-textarea"
              value={fields[meta.key]}
              onChange={(e) =>
                onChange({ ...fields, [meta.key]: e.target.value })
              }
              placeholder={meta.placeholder}
              rows={meta.rows}
            />
          </div>
        );
      })}
    </div>
  );
}
