"use client";

import { useEffect, useState } from "react";

import { listRuntimeHosts } from "../../lib/api/choruz-api";
import type { RuntimeHost } from "../../lib/remote/remote-control";
import { DriverSelect } from "./driver-select";
import { HarnessAccountPicker } from "./harness-account-picker";
import { Modal } from "../ui/modal";

type Props = {
  companyId: string;
  sessionToken: string;
  multiHarnessAccounts: boolean;
  onMultiHarnessAccountsChange: (enabled: boolean) => Promise<void>;
  onClose: () => void;
};

/** Device-level view of the login each harness has, login repair, exact usage, and the company's multi-account switch. */
export function HarnessAccountsModal({ companyId, sessionToken, multiHarnessAccounts, onMultiHarnessAccountsChange, onClose }: Props) {
  const [hosts, setHosts] = useState<RuntimeHost[]>([]);
  const [hostId, setHostId] = useState("");
  const [driver, setDriver] = useState<"claude_terminal" | "codex_terminal">("claude_terminal");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The checkbox reflects the click at once; the company answers after the PATCH.
  const [enabled, setEnabled] = useState(multiHarnessAccounts);
  useEffect(() => { setEnabled(multiHarnessAccounts); }, [multiHarnessAccounts]);

  useEffect(() => {
    let cancelled = false;
    void listRuntimeHosts(sessionToken, companyId)
      .then((items) => { if (!cancelled) setHosts(items); })
      .catch(() => { if (!cancelled) setHosts([]); });
    return () => { cancelled = true; };
  }, [companyId, sessionToken]);

  const toggle = async (next: boolean) => {
    setEnabled(next);
    setSaving(true);
    setError(null);
    try {
      await onMultiHarnessAccountsChange(next);
    } catch (caught) {
      setEnabled(!next);
      setError(caught instanceof Error ? caught.message : "Unable to update the company");
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      eyebrow="Device setup"
      title="Harness Accounts"
      description="Choruz verifies the login each device already has and shows its plan and exact usage. Credentials stay on that device."
      onClose={onClose}
      className="harness-accounts-card"
    >
      <div className="modal-form">
        <label className="harness-accounts-toggle">
          <input
            type="checkbox"
            checked={enabled}
            disabled={saving}
            onChange={(event) => void toggle(event.target.checked)}
          />
          Allow multiple accounts in this company
        </label>
        <p className="field-hint">
          {enabled
            ? "Add more sign-ins below and choose one for each Agent. An Agent without a choice uses the device's own login."
            : "Every Agent uses the login its device already has. Turn this on to sign in to more accounts."}
        </p>
        {error ? <p className="create-agent-warning" role="alert">{error}</p> : null}
        <label>
          Device
          <select aria-label="Account device" value={hostId} onChange={(event) => setHostId(event.target.value)}>
            <option value="">This computer</option>
            {hosts.map((host) => (
              <option key={host.id} value={host.id} disabled={host.status !== "online"}>
                {host.name}{host.status === "online" ? "" : " (offline)"}
              </option>
            ))}
          </select>
        </label>
        <label>
          Harness
          <DriverSelect aria-label="Account harness" value={driver} onChange={(next) => setDriver(next as typeof driver)} drivers={["claude_terminal", "codex_terminal"]} />
        </label>
        <HarnessAccountPicker
          key={`${companyId}:${hostId}:${driver}`}
          companyId={companyId}
          runtimeHostId={hostId}
          driver={driver}
          value=""
          onChange={() => {}}
          mode="manage"
          allowMultiple={enabled}
        />
      </div>
    </Modal>
  );
}
