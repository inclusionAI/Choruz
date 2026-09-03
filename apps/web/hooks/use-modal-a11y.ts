"use client";

import { useEffect, useRef, type RefObject } from "react";

const FOCUSABLE =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

const modalStack: symbol[] = [];
let bodyLockCount = 0;
let bodyOverflowBeforeLock = "";

/**
 * Wires up the four accessibility behaviours every modal should have
 * (and which Choruz's hand-written modals were missing):
 *
 * 1. Esc key closes the modal.
 * 2. Body scroll is locked while the modal is open — prevents the
 *    page underneath from scrolling when the user wheels inside the
 *    overlay.
 * 3. Focus moves into the modal on open — focused on the first
 *    focusable descendant of `containerRef`, falling back to the
 *    container itself.
 * 4. Tab / Shift+Tab cycles within the modal (focus trap), so
 *    keyboard users can't tab back into the obscured page behind.
 * 5. Returns focus to the previously-focused element on unmount.
 *
 * Usage: render the modal conditionally; when the JSX tree containing
 * useModalA11y is mounted, the hook is "open"; unmounting (e.g.
 * `{open && <Modal/>}` flipping false) cleans up automatically.
 *
 * No external dependency — Radix Dialog gives this and more, but the
 * app only needs these behaviours, and `components/ui/modal.tsx` is the one
 * caller.
 */
export function useModalA11y(
  containerRef: RefObject<HTMLElement | null>,
  onClose: () => void,
) {
  const modalIdRef = useRef(Symbol("choruz-modal"));
  const openerRef = useRef<HTMLElement | null>(
    typeof document === "undefined" ? null : document.activeElement as HTMLElement | null,
  );
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  // Esc to close + focus trap on Tab.
  useEffect(() => {
    const modalId = modalIdRef.current;
    modalStack.push(modalId);
    const handleKey = (e: KeyboardEvent) => {
      if (modalStack.at(-1) !== modalId) return;
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onCloseRef.current();
        return;
      }
      if (e.key !== "Tab") return;
      const root = containerRef.current;
      if (!root) return;
      const focusables = root.querySelectorAll<HTMLElement>(FOCUSABLE);
      if (focusables.length === 0) {
        // Nothing enabled to cycle on (every control disabled while an
        // action runs): keep focus on the dialog itself.
        e.preventDefault();
        root.focus();
        return;
      }
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      const active = document.activeElement as HTMLElement | null;
      if (e.shiftKey) {
        if (active === first || !root.contains(active)) {
          e.preventDefault();
          last.focus();
        }
      } else if (active === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", handleKey);
    return () => {
      document.removeEventListener("keydown", handleKey);
      const index = modalStack.lastIndexOf(modalId);
      if (index >= 0) modalStack.splice(index, 1);
    };
  }, [containerRef]);

  // Body scroll lock + restore previous focus on unmount.
  useEffect(() => {
    const previouslyFocused = openerRef.current;
    if (bodyLockCount === 0) {
      bodyOverflowBeforeLock = document.body.style.overflow;
    }
    bodyLockCount += 1;
    document.body.style.overflow = "hidden";

    // Move focus into the modal on next tick (let React finish painting
    // so refs are populated and disabled buttons are real). Controls marked
    // data-autofocus-skip (the header close button) yield to the first
    // real control, but still take focus when they are all there is, so
    // the trap has a control to cycle on instead of the dialog root.
    const t = setTimeout(() => {
      const root = containerRef.current;
      if (!root) return;
      const firstFocusable =
        root.querySelector<HTMLElement>(`:is(${FOCUSABLE}):not([data-autofocus-skip])`) ??
        root.querySelector<HTMLElement>(FOCUSABLE);
      (firstFocusable ?? root).focus();
    }, 0);

    return () => {
      clearTimeout(t);
      bodyLockCount = Math.max(0, bodyLockCount - 1);
      if (bodyLockCount === 0) {
        document.body.style.overflow = bodyOverflowBeforeLock;
      }
      if (previouslyFocused?.isConnected) {
        previouslyFocused.focus();
      } else {
        document.querySelector<HTMLElement>("[data-modal-return-focus]")?.focus();
      }
    };
  }, [containerRef]);
}
