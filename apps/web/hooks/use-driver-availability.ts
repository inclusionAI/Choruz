"use client";

import { useEffect, useState } from "react";
import type { ClientDriverAvailabilityItem } from "../lib/agents/create-agent-template-flow";
import { transportFetch } from "../lib/api/transport";

/**
 * Loads installed drivers whenever the selected device changes.
 * `loaded` stays false on failure so callers can block a launch until the
 * answer is known; `error` carries the reason.
 */
export function useDriverAvailability(runtimeHostId = "") {
  const [availability, setAvailability] = useState<ClientDriverAvailabilityItem[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoaded(false);
    setError(null);
    setAvailability([]);
    const query = runtimeHostId ? `?runtime_host_id=${encodeURIComponent(runtimeHostId)}` : "";
    void transportFetch(`/api/drivers/availability${query}`)
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
  }, [runtimeHostId]);

  return { availability, loaded, error };
}
