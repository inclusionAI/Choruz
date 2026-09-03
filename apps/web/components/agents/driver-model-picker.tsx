"use client";

import { useEffect, useId, useState } from "react";

import type { DriverModel, DriverModelDiscovery } from "../../lib/drivers/driver-models";
import type { DriverId } from "../../lib/groups/team-templates";
import { transportFetch } from "../../lib/api/transport";

export function DriverModelPicker({
  driver,
  model,
  onChange,
  label = "Model",
  disabled = false,
  accountModels,
}: {
  driver: DriverId;
  model: string;
  onChange: (model: string) => void;
  label?: string;
  disabled?: boolean;
  accountModels?: DriverModel[];
}) {
  const id = useId().replaceAll(":", "");
  const [result, setResult] = useState<DriverModelDiscovery | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (driver === "webhook_agent" || accountModels) {
      setResult(null);
      setLoading(false);
      return;
    }
    const controller = new AbortController();
    setLoading(true);
    setResult(null);
    void transportFetch(`/api/drivers/models?driver_type=${encodeURIComponent(driver)}`, {
      signal: controller.signal,
    })
      .then(async (response) => {
        if (!response.ok) throw new Error(`Model scan failed (${response.status})`);
        return (await response.json()) as DriverModelDiscovery;
      })
      .then((discovery) => {
        if (!controller.signal.aborted && discovery.driverId === driver) {
          setResult(discovery);
        }
      })
      .catch((error) => {
        if (!controller.signal.aborted && (error as Error).name !== "AbortError") {
          setResult({
            driverId: driver,
            status: "unavailable",
            models: [],
            message: "Model discovery is unavailable. You can still enter an exact model ID.",
          });
        }
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [accountModels, driver]);

  if (driver === "webhook_agent") return null;

  return (
    <label>
      {label}
      <input
        aria-label={label}
        list={`driver-models-${id}`}
        value={model}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
        placeholder="Default (harness setting)"
        autoComplete="off"
        spellCheck={false}
      />
      <datalist id={`driver-models-${id}`}>
        {(accountModels ?? result?.models ?? []).map((item) => (
          <option key={item.id} value={item.id}>
            {item.label === item.id ? item.description : [item.label, item.description].filter(Boolean).join(" — ")}
          </option>
        ))}
      </datalist>
      <span className="field-hint" role="status">
        {accountModels
          ? `${accountModels.length} models verified for the selected account.`
          : loading
          ? "Scanning models from the installed harness…"
          : result?.message ?? "Leave blank to use the harness default."}
      </span>
    </label>
  );
}
