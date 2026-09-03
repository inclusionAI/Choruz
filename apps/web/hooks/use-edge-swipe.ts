"use client";

import { useEffect, useRef, type RefObject } from "react";

const SWIPE_THRESHOLD = 50;
const EDGE_ZONE = 30;
const MAX_SWIPE_MS = 500;

/**
 * Touch gestures for the mobile drawer: a quick rightward swipe from the
 * left edge opens it, a leftward swipe closes it. Vertical or slow
 * gestures are ignored. `setOpen` should be a state setter (stable), so
 * the listeners bind once.
 */
export function useEdgeSwipe({
  ref,
  setOpen,
}: {
  ref: RefObject<HTMLElement | null>;
  setOpen: (open: boolean) => void;
}) {
  const touchStartRef = useRef<{ x: number; y: number; time: number } | null>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    const onTouchStart = (e: TouchEvent) => {
      const touch = e.touches[0];
      touchStartRef.current = { x: touch.clientX, y: touch.clientY, time: Date.now() };
    };

    const onTouchEnd = (e: TouchEvent) => {
      if (!touchStartRef.current) return;
      const touch = e.changedTouches[0];
      const dx = touch.clientX - touchStartRef.current.x;
      const dy = touch.clientY - touchStartRef.current.y;
      const elapsed = Date.now() - touchStartRef.current.time;
      const startX = touchStartRef.current.x;
      touchStartRef.current = null;

      if (elapsed > MAX_SWIPE_MS || Math.abs(dy) > Math.abs(dx)) return;
      if (dx > SWIPE_THRESHOLD && startX < EDGE_ZONE) setOpen(true);
      else if (dx < -SWIPE_THRESHOLD) setOpen(false);
    };

    el.addEventListener("touchstart", onTouchStart, { passive: true });
    el.addEventListener("touchend", onTouchEnd, { passive: true });
    return () => {
      el.removeEventListener("touchstart", onTouchStart);
      el.removeEventListener("touchend", onTouchEnd);
    };
  }, [ref, setOpen]);
}
