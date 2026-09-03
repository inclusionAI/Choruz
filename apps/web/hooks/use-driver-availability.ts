"use client";

import { useEffect, useState } from "react";
import type { ClientDriverAvailabilityItem } from "../lib/agents/create-agent-template-flow";
import { transportFetch } from "../lib/api/transport";

/**
 * Loads which drivers are installed on this machine, once per mount.
 * `loaded` stays false on failure so callers can block a launch until the
 * answer is known; `error` carries the reason.
 */
export function useDriverAvailability() {
  const [availability, setAvailability] = useState<ClientDriverAvailabilityItem[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void transportFetch("/api/drivers/availability")
      .then((res) => {
        if (!res.ok) throw new Error("Failed to load driver availability.");
        return res.json() as Promise<{ drivers?: ClientDriverAvailabilityItem[] }>;
      })
      .then((data) => {
        if (cancelled) return;
        setAvailability(data.drivers ?? []);
        setLoaded(true);
      })
      .catch((err) => {
        if (cancelled) return;
        setAvailability([]);
        setError(err instanceof Error ? err.message : "Failed to load driver availability.");
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return { availability, loaded, error };
}
