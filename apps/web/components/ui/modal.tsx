"use client";

import { useId, useRef, type ReactNode } from "react";
import { X } from "lucide-react";
import { useModalA11y } from "../../hooks/use-modal-a11y";

type ModalProps = {
  title: ReactNode;
  onClose: () => void;
  /** One line under the title explaining what the dialog is for; becomes the dialog's description. */
  description?: ReactNode;
  /** Small label above the title (kept out of the `h2` so it stays out of the accessible name). */
  eyebrow?: ReactNode;
  /** Extra header controls, rendered before the close button. */
  headerActions?: ReactNode;
  /** Size / skin classes added to `.modal-card`. */
  className?: string;
  /**
   * `padded` (default) is a form card. `flush` drops the card padding so
   * the header and any `.modal-actions` footer draw their own hairlines;
   * the caller lays out and scrolls the body.
   */
  layout?: "padded" | "flush";
  closeLabel?: string;
  /** Block every way out (close button, backdrop, Esc) while an action is in flight. */
  closeDisabled?: boolean;
  /** Overrides the description as the `aria-describedby` target, e.g. an error message. */
  describedBy?: string;
  children: ReactNode;
};

/**
 * The one dialog shell: backdrop, card, header with title and close
 * button, and the keyboard / focus behaviour from `useModalA11y`. Body
 * and footer are children, since multi-step dialogs render a footer per
 * step.
 */
export function Modal({
  title,
  onClose,
  description,
  eyebrow,
  headerActions,
  className,
  layout = "padded",
  closeLabel = "Close",
  closeDisabled = false,
  describedBy,
  children,
}: ModalProps) {
  const cardRef = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const descriptionId = useId();
  const requestClose = closeDisabled ? () => {} : onClose;
  useModalA11y(cardRef, requestClose);

  const cardClass = ["modal-card", layout === "flush" && "modal-card--flush", className]
    .filter(Boolean)
    .join(" ");

  return (
    <div
      className="modal-overlay"
      onClick={(event) => {
        if (event.target === event.currentTarget) requestClose();
      }}
    >
      <div
        ref={cardRef}
        className={cardClass}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={describedBy ?? (description ? descriptionId : undefined)}
        tabIndex={-1}
      >
        <header className="modal-header">
          <div className="modal-heading">
            {eyebrow && <span className="eyebrow">{eyebrow}</span>}
            <h2 id={titleId}>{title}</h2>
            {description && (
              <p id={descriptionId} className="modal-description">
                {description}
              </p>
            )}
          </div>
          {headerActions}
          {/* Skipped for initial focus so the first form control gets it. */}
          <button
            type="button"
            className="modal-close"
            onClick={requestClose}
            aria-label={closeLabel}
            disabled={closeDisabled}
            data-autofocus-skip
          >
            <X size={16} aria-hidden="true" />
          </button>
        </header>
        {children}
      </div>
    </div>
  );
}
