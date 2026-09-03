"use client";

import type { KeyboardEvent } from "react";
import type { PanelResize } from "../../hooks/use-panel-resize";

const KEYBOARD_STEP_PX = 20;

/**
 * The drag handle for a resizable side panel: mouse drag plus Arrow
 * Left/Right in screen direction (moving the handle right widens a
 * left-anchored panel and narrows a right-anchored one).
 */
export function ResizeHandle({ resize, label }: { resize: PanelResize; label: string }) {
  const onKeyDown = (e: KeyboardEvent) => {
    const direction = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!direction) return;
    e.preventDefault();
    resize.nudge(direction * KEYBOARD_STEP_PX * (resize.anchor === "left" ? 1 : -1));
  };
  return (
    <div
      className={`sidebar-resize-handle${resize.resizing ? " dragging" : ""}`}
      onMouseDown={resize.onMouseDown}
      onKeyDown={onKeyDown}
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      tabIndex={0}
    />
  );
}
