"use client";

import { useEffect, useMemo, useState } from "react";

import type { ChatMessage, Principal } from "../../lib/api/choruz-types";
import { defaultTaskTitleFromMessage } from "../../lib/channel-tasks/channel-task-create";
import { Modal } from "../ui/modal";

type ChannelTaskCreateModalProps = {
  message: ChatMessage;
  visibleAssignees: Principal[];
  submitting: boolean;
  error: string | null;
  onClose: () => void;
  onSubmit: (payload: {
    title: string;
    assigneePrincipalId: string;
    contextLabel: string | null;
  }) => void;
};

export function ChannelTaskCreateModal({
  message,
  visibleAssignees,
  submitting,
  error,
  onClose,
  onSubmit,
}: ChannelTaskCreateModalProps) {
  const initialTitle = useMemo(() => defaultTaskTitleFromMessage(message.content), [message.content]);
  const [title, setTitle] = useState(initialTitle);
  const [assigneePrincipalId, setAssigneePrincipalId] = useState("");
  const [contextLabel, setContextLabel] = useState("");

  useEffect(() => {
    setTitle(initialTitle);
    setAssigneePrincipalId("");
    setContextLabel("");
  }, [message.id, initialTitle]);

  const canSubmit = title.trim().length > 0 && assigneePrincipalId.length > 0 && !submitting;

  return (
    <Modal title="Create task" onClose={onClose} className="channel-task-create-modal">
      <div className="modal-form">
        <label>
          <span>Title</span>
          <input
            value={title}
            onChange={(event) => setTitle(event.target.value)}
            maxLength={140}
            disabled={submitting}
          />
        </label>
        <label>
          <span>Assignee</span>
          <select
            value={assigneePrincipalId}
            onChange={(event) => setAssigneePrincipalId(event.target.value)}
            disabled={submitting}
            required
          >
            <option value="" disabled>Select assignee</option>
            {visibleAssignees.map((assignee) => (
              <option key={assignee.id} value={assignee.id}>
                {assignee.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>Context</span>
          <input
            value={contextLabel}
            onChange={(event) => setContextLabel(event.target.value)}
            maxLength={80}
            disabled={submitting}
          />
        </label>
        <div className="channel-task-create-source">
          {message.content.length > 160 ? `${message.content.slice(0, 157).trimEnd()}…` : message.content}
        </div>
        {error ? (
          <p className="modal-form-error" role="alert" aria-live="polite">
            {error}
          </p>
        ) : null}
      </div>
      <div className="modal-actions">
        <button className="btn-cancel" type="button" onClick={onClose} disabled={submitting}>
          Cancel
        </button>
        <button
          className="btn-primary"
          type="button"
          onClick={() =>
            onSubmit({
              title: title.trim(),
              assigneePrincipalId,
              contextLabel: contextLabel.trim() ? contextLabel.trim() : null,
            })
          }
          disabled={!canSubmit}
        >
          {submitting ? "Creating…" : "Create"}
        </button>
      </div>
    </Modal>
  );
}
