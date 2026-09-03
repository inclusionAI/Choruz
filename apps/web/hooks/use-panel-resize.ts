"use client";

import { useCallback, useEffect, useRef, useState, type MouseEvent } from "react";

type Options = {
  /** localStorage key the width is restored from and persisted to. */
  storageKey: string;
  initial: number;
  min: number;
  max: number;
  /** Which edge stays put: a left-anchored panel grows when dragged right. */
  anchor: "left" | "right";
};

/**
 * Drag-to-resize for a side panel: width state, a `resizing` flag for
 * styling, the mouse-down handler for the drag handle, and a keyboard
 * `nudge` (see `ResizeHandle`, which wires both). The width is restored from localStorage after mount (never
 * during render, to keep server and client markup identical) and
 * persisted when a drag or nudge ends.
 */
export type PanelResize = ReturnType<typeof usePanelResize>;

export function usePanelResize({ storageKey, initial, min, max, anchor }: Options) {
  const [width, setWidth] = useState(initial);
  const [resizing, setResizing] = useState(false);
  const dragRef = useRef<{ startX: number; startWidth: number } | null>(null);

  useEffect(() => {
    try {
      const saved = parseInt(localStorage.getItem(storageKey) ?? "", 10);
      if (saved >= min && saved <= max) setWidth(saved);
    } catch { /* ignore */ }
  }, [storageKey, min, max]);

  const clamp = useCallback((value: number) => Math.min(max, Math.max(min, value)), [min, max]);

  const persist = useCallback(
    (value: number) => {
      try { localStorage.setItem(storageKey, String(value)); } catch { /* ignore */ }
    },
    [storageKey],
  );

  const onMouseDown = useCallback(
    (e: MouseEvent) => {
      e.preventDefault();
      dragRef.current = { startX: e.clientX, startWidth: width };
      setResizing(true);

      const onMouseMove = (ev: globalThis.MouseEvent) => {
        if (!dragRef.current) return;
        const travel = ev.clientX - dragRef.current.startX;
        const delta = anchor === "left" ? travel : -travel;
        setWidth(clamp(dragRef.current.startWidth + delta));
      };

      const onMouseUp = () => {
        document.removeEventListener("mousemove", onMouseMove);
        document.removeEventListener("mouseup", onMouseUp);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
        setResizing(false);
        setWidth((current) => {
          persist(current);
          return current;
        });
        dragRef.current = null;
      };

      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
      document.addEventListener("mousemove", onMouseMove);
      document.addEventListener("mouseup", onMouseUp);
    },
    [anchor, clamp, persist, width],
  );

  const nudge = useCallback(
    (delta: number) => {
      const next = clamp(width + delta);
      setWidth(next);
      persist(next);
    },
    [clamp, persist, width],
  );

  return { width, resizing, anchor, onMouseDown, nudge };
}
