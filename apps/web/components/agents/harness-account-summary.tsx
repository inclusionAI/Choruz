"use client";

import { useEffect, useState } from "react";

import { displayUsageWindows } from "../../lib/agents/harness-account-display";
import type { HarnessAccount } from "../../lib/agents/harness-accounts";
import { transportFetch } from "../../lib/api/transport";

export function HarnessAccountSummary({ accountId, companyId }: { accountId: string; companyId: string }) {
  const [account, setAccount] = useState<HarnessAccount | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    const query = new URLSearchParams({ company_id: companyId });
    void transportFetch(`/api/harness-accounts/${encodeURIComponent(accountId)}?${query}`, { signal: controller.signal })
      .then(async (response) => response.ok ? response.json() as Promise<HarnessAccount> : null)
      .then((value) => setAccount(value))
      .catch((error: unknown) => {
        if (!(error instanceof DOMException && error.name === "AbortError")) setAccount(null);
      });
    return () => controller.abort();
  }, [accountId, companyId]);

  if (!account || account.status !== "active") return null;
  return (
    <div className="harness-account-summary">
      <strong>Account</strong>{" "}
      <span>{account.name}{account.subscriptionType ? ` · ${account.subscriptionType}` : ""}</span>
      <div className="harness-account-summary-windows">
        {displayUsageWindows(account.driverType, account.usage.windows).map((window) => (
          <span key={window.id} title={window.resetsAt ? `Resets ${new Date(window.resetsAt).toLocaleString("en-US")}` : undefined}>
            {window.label} {window.remainingPercent}% left
          </span>
        ))}
      </div>
      {account.probedAt ? <small>Exact usage updated {new Date(account.probedAt).toLocaleString("en-US")}</small> : null}
    </div>
  );
}
