"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { displayUsageWindows } from "../../lib/agents/harness-account-display";
import type { HarnessAccount } from "../../lib/agents/harness-accounts";
import type { DriverId } from "../../lib/groups/team-templates";
import { transportFetch } from "../../lib/api/transport";
import { TerminalView } from "../runtime/terminal-view";

type ApiErrorBody = { error?: string | { detail?: string } };

function apiErrorMessage(body: ApiErrorBody, fallback: string): string {
  if (typeof body.error === "string") return body.error;
  return body.error?.detail || fallback;
}

function startHarnessLogin(accountId: string, companyId: string): Promise<HarnessLogin> {
  return transportFetch(`/api/harness-accounts/${encodeURIComponent(accountId)}/login?company_id=${encodeURIComponent(companyId)}`, { method: "POST" })
    .then(async (response) => {
      const body = await response.json() as HarnessLogin & ApiErrorBody;
      if (!response.ok) throw new Error(apiErrorMessage(body, "Unable to start the sign-in"));
      return body;
    });
}

function isDefaultAccount(account: HarnessAccount): boolean {
  return account.profileKind === "default";
}

export function HarnessAccountPicker({
  companyId,
  runtimeHostId,
  driver,
  value,
  onChange,
  mode = "select",
  allowMultiple = false,
  sessionToken,
}: {
  companyId: string | null | undefined;
  runtimeHostId: string;
  driver: DriverId;
  value: string;
  onChange: (account: HarnessAccount | null) => void;
  /** Account setup belongs in the device-level manager; Create Agent selects only. */
  mode?: "select" | "manage";
  /** The company's multi-account switch: adding, choosing and removing device sign-ins. */
  allowMultiple?: boolean;
  sessionToken?: string;
}) {
  const supported = driver === "claude_terminal" || driver === "codex_terminal";
  const [accounts, setAccounts] = useState<HarnessAccount[]>([]);
  const [loading, setLoading] = useState(false);
  const [verifyingDefault, setVerifyingDefault] = useState(false);
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const [loginAccount, setLoginAccount] = useState<HarnessAccount | null>(null);
  const [error, setError] = useState<string | null>(null);
  const contextKey = `${companyId ?? ""}\0${runtimeHostId}\0${driver}`;
  const contextKeyRef = useRef(contextKey);
  const loadSequenceRef = useRef(0);
  const onChangeRef = useRef(onChange);
  const valueRef = useRef(value);
  contextKeyRef.current = contextKey;
  onChangeRef.current = onChange;
  valueRef.current = value;

  const upsert = useCallback((account: HarnessAccount) => {
    setAccounts((current) => [...current.filter((item) => item.id !== account.id), account]);
  }, []);

  const load = useCallback(async () => {
    const sequence = ++loadSequenceRef.current;
    const requestedContext = contextKey;
    const current = () => sequence === loadSequenceRef.current && requestedContext === contextKeyRef.current;
    if (!supported || !companyId) {
      setAccounts([]);
      setLoading(false);
      setError(null);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const query = new URLSearchParams({ company_id: companyId });
      if (runtimeHostId) query.set("runtime_host_id", runtimeHostId);
      const response = await transportFetch(`/api/harness-accounts?${query}`);
      const body = await response.json() as { accounts?: HarnessAccount[]; error?: string };
      if (!response.ok) throw new Error(body.error || "Unable to load harness accounts");
      if (!current()) return;
      setAccounts((body.accounts ?? []).filter((account) => account.driverType === driver));
    } catch (caught) {
      if (!current()) return;
      setAccounts([]);
      setError(caught instanceof Error ? caught.message : "Unable to load harness accounts");
      setLoading(false);
      return;
    }
    setLoading(false);
    // The device's own login is registered on demand; the manager verifies
    // it here, Create Agent leaves an unverified one to provisioning.
    setVerifyingDefault(mode === "manage");
    try {
      const response = await transportFetch("/api/harness-accounts/default", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          company_id: companyId,
          ...(runtimeHostId ? { runtime_host_id: runtimeHostId } : {}),
          driver_type: driver,
          probe: mode === "manage",
          reactivate_removed: !allowMultiple,
        }),
      });
      if (response.status === 204) return;
      const account = await response.json() as HarnessAccount & { error?: string };
      if (!response.ok) throw new Error(account.error || "Unable to read the device's login");
      if (!current()) return;
      upsert(account);
      if (mode === "select" && !valueRef.current && account.status === "active") onChangeRef.current(account);
    } catch (caught) {
      if (!current()) return;
      setError(caught instanceof Error ? caught.message : "Unable to read the device's login");
    } finally {
      if (current()) setVerifyingDefault(false);
    }
  }, [allowMultiple, companyId, contextKey, driver, mode, runtimeHostId, supported, upsert]);

  useEffect(() => { void load(); }, [load]);

  const selected = useMemo(() => accounts.find((account) => account.id === value) ?? null, [accounts, value]);
  const sorted = useMemo(
    () => [...accounts].sort((left, right) => Number(isDefaultAccount(right)) - Number(isDefaultAccount(left)) || left.name.localeCompare(right.name)),
    [accounts],
  );
  if (!supported) return null;
  const harnessLabel = driver === "claude_terminal" ? "Claude Code" : "Codex";
  const deviceWord = runtimeHostId ? "host" : "computer";

  const probe = async (account: HarnessAccount) => {
    if (!companyId) return;
    setLoading(true);
    setError(null);
    try {
      const response = await transportFetch(`/api/harness-accounts/${encodeURIComponent(account.id)}/probe?company_id=${encodeURIComponent(companyId)}`, { method: "POST" });
      const probed = await response.json() as HarnessAccount & { error?: string };
      if (!response.ok) throw new Error(probed.error || "Account probe failed");
      upsert(probed);
      onChange(probed);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Account probe failed");
      await load();
    } finally {
      setLoading(false);
    }
  };

  const addAccount = async () => {
    if (!companyId || !name.trim()) return;
    setLoading(true);
    setError(null);
    try {
      const createdResponse = await transportFetch("/api/harness-accounts", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          company_id: companyId,
          ...(runtimeHostId ? { runtime_host_id: runtimeHostId } : {}),
          driver_type: driver,
          profile_kind: "isolated",
          name: name.trim(),
        }),
      });
      const created = await createdResponse.json() as HarnessAccount & { error?: string };
      if (!createdResponse.ok) throw new Error(created.error || "Unable to add account");
      upsert(created);
      setAdding(false);
      setName("");
      setLoginAccount(created);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Unable to add account");
    } finally {
      setLoading(false);
    }
  };

  const removeAccount = async (account: HarnessAccount) => {
    if (!companyId) return;
    setLoading(true);
    setError(null);
    try {
      const response = await transportFetch(`/api/harness-accounts/${encodeURIComponent(account.id)}?company_id=${encodeURIComponent(companyId)}`, { method: "DELETE" });
      const body = await response.json() as { error?: string };
      if (!response.ok) throw new Error(body?.error || "Unable to remove account");
      setAccounts((current) => current.filter((item) => item.id !== account.id));
      if (value === account.id) onChange(null);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Unable to remove account");
    } finally {
      setLoading(false);
    }
  };

  const statusLabel = (account: HarnessAccount) => {
    if (account.status === "active") return "";
    if (isDefaultAccount(account) && verifyingDefault) return " · verifying";
    return ` · ${account.status}`;
  };

  return (
    <div className="harness-account-picker">
      {mode === "select" ? (
        <label>
          Harness account
          <select
            aria-label="Harness account"
            value={value}
            disabled={loading}
            onChange={(event) => onChange(accounts.find((account) => account.id === event.target.value) ?? null)}
          >
            <option value="">Login already on this {deviceWord}</option>
            {sorted.map((account) => (
              <option key={account.id} value={account.id} disabled={account.status !== "active"}>
                {account.name}{account.subscriptionType ? ` · ${account.subscriptionType}` : ""}{account.status === "active" ? "" : ` (${account.status})`}
              </option>
            ))}
          </select>
        </label>
      ) : (
        <section className="harness-account-list" aria-label="Harness accounts">
          {sorted.length === 0 && !loading && !verifyingDefault ? <p className="field-hint">No {harnessLabel} accounts are configured for this {deviceWord}.</p> : null}
          {sorted.map((account) => (
            <div className="harness-account-record" key={account.id}>
              <div>
                <strong>{account.name}</strong>
                <span>{account.subscriptionType ? ` · ${account.subscriptionType}` : ""}{statusLabel(account)}</span>
              </div>
              {isDefaultAccount(account) ? (
                <p className="field-hint">The {harnessLabel} login this {deviceWord} already has.</p>
              ) : null}
              {account.status === "active" ? (
                <AccountUsage account={account} loading={loading} onRefresh={() => void probe(account)} />
              ) : null}
              {account.status !== "active" && !(isDefaultAccount(account) && verifyingDefault) ? (
                <div className="harness-account-repair">
                  {account.lastError ? <p className="field-hint">{account.lastError}</p> : null}
                  <button type="button" className="btn-secondary" disabled={loading} onClick={() => setLoginAccount(account)}>Sign in</button>
                  {isDefaultAccount(account) ? (
                    <button type="button" className="btn-secondary" disabled={loading} onClick={() => void probe(account)}>Verify again</button>
                  ) : null}
                </div>
              ) : null}
              {allowMultiple ? (
                <button type="button" className="btn-secondary harness-account-remove-button" disabled={loading} onClick={() => void removeAccount(account)}>Remove account</button>
              ) : null}
            </div>
          ))}
        </section>
      )}
      {mode === "select" && selected ? <p className="field-hint">This Agent will use the selected account&apos;s verified models.</p> : null}
      {mode === "manage" && loginAccount ? (
        <HarnessLoginPanel
          sessionToken={sessionToken}
          account={loginAccount}
          companyId={companyId!}
          onVerified={(account) => {
            upsert(account);
            setLoginAccount(null);
            onChange(account);
          }}
          onClose={() => setLoginAccount(null)}
        />
      ) : null}
      {mode === "manage" && allowMultiple ? (adding ? (
        <div className="harness-account-add">
          <label>
            Account label
            <input value={name} onChange={(event) => setName(event.target.value)} placeholder="Work account" maxLength={80} />
          </label>
          <p className="field-hint">
            {driver === "claude_terminal" ? "Open Claude Code to sign in directly in its terminal." : `Choruz starts the official ${harnessLabel} browser sign-in.`} Credentials stay on the {deviceWord}; removing the account later hides it in Choruz and leaves them there.
          </p>
          <div className="modal-actions compact">
            <button type="button" className="btn-secondary" onClick={() => setAdding(false)}>Cancel</button>
            <button type="button" className="btn-primary" disabled={loading || !name.trim()} onClick={() => void addAccount()}>
              {loading ? "Adding…" : driver === "claude_terminal" ? "Open Claude Code to sign in" : "Add and sign in"}
            </button>
          </div>
        </div>
      ) : (
        <button type="button" className="btn-secondary harness-account-add-button" onClick={() => setAdding(true)}>Add account</button>
      )) : null}
      {error ? <p className="create-agent-warning" role="alert">{error}</p> : null}
    </div>
  );
}

type HarnessLogin = {
  id: string;
  state: "queued" | "awaiting_browser" | "authorizing" | "verified" | "failed" | "cancelled" | "expired";
  authorization_url: string | null;
  user_code: string | null;
  error: string | null;
};

/** Starts the official harness sign-in for a local or remote account and polls it until it verifies. */
function HarnessLoginPanel({ account, companyId, sessionToken, onVerified, onClose }: {
  account: HarnessAccount;
  companyId: string;
  sessionToken?: string;
  onVerified: (account: HarnessAccount) => void;
  onClose: () => void;
}) {
  const harnessLabel = account.driverType === "claude_terminal" ? "Claude Code" : "Codex";
  const [login, setLogin] = useState<HarnessLogin | null>(null);
  const [code, setCode] = useState("");
  const onVerifiedRef = useRef(onVerified);
  onVerifiedRef.current = onVerified;
  const [error, setError] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const isClaude = account.driverType === "claude_terminal";

  const complete = async () => {
    if (!login) return;
    setChecking(true);
    setError(null);
    try {
      const response = await transportFetch(`/api/harness-accounts/${encodeURIComponent(account.id)}/login/${encodeURIComponent(login.id)}/complete?company_id=${encodeURIComponent(companyId)}`, { method: "POST" });
      if (!response.ok) {
        const body = await response.json() as ApiErrorBody;
        throw new Error(apiErrorMessage(body, "Claude Code is not signed in"));
      }
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Unable to check sign-in");
    } finally { setChecking(false); }
  };

  useEffect(() => {
    let cancelled = false;
    const start = async () => {
      try {
        const body = await startHarnessLogin(account.id, companyId);
        if (!cancelled) setLogin(body);
      } catch (reason) {
        if (!cancelled) setError(reason instanceof Error ? reason.message : "Unable to start the sign-in");
      }
    };
    void start();
    return () => { cancelled = true; };
  }, [account.id, companyId]);

  useEffect(() => {
    if (!login || ["verified", "failed", "expired", "cancelled"].includes(login.state)) return;
    let cancelled = false;
    const poll = async () => {
      try {
        const response = await transportFetch(`/api/harness-accounts/${encodeURIComponent(account.id)}/login/${encodeURIComponent(login.id)}?company_id=${encodeURIComponent(companyId)}`, { cache: "no-store" });
        const body = await response.json() as HarnessLogin & ApiErrorBody;
        if (!response.ok) throw new Error(apiErrorMessage(body, "Unable to read the sign-in"));
        if (cancelled) return;
        setLogin(body);
        if (body.state === "verified") {
          const accountResponse = await transportFetch(`/api/harness-accounts/${encodeURIComponent(account.id)}?company_id=${encodeURIComponent(companyId)}`, { cache: "no-store" });
          const verified = await accountResponse.json() as HarnessAccount;
          if (accountResponse.ok) onVerifiedRef.current(verified);
        }
      } catch (reason) {
        if (!cancelled) setError(reason instanceof Error ? reason.message : "Unable to read the sign-in");
      }
    };
    const interval = setInterval(() => { void poll(); }, 1_000);
    void poll();
    return () => { cancelled = true; clearInterval(interval); };
  }, [account.id, companyId, login?.id, login?.state]);

  const OPEN_STATES = ["queued", "awaiting_browser", "authorizing"];
  const cancel = async () => {
    if (login && OPEN_STATES.includes(login.state)) {
      await transportFetch(`/api/harness-accounts/${encodeURIComponent(account.id)}/login/${encodeURIComponent(login.id)}/cancel?company_id=${encodeURIComponent(companyId)}`, { method: "POST" }).catch(() => {});
    }
    onClose();
  };

  const submitCode = async () => {
    if (!login || !code.trim()) return;
    setError(null);
    const response = await transportFetch(`/api/harness-accounts/${encodeURIComponent(account.id)}/login/${encodeURIComponent(login.id)}/callback?company_id=${encodeURIComponent(companyId)}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ code: code.trim() }),
    });
    if (!response.ok) {
      const body = await response.json().catch(() => ({ error: "Unable to submit the authorization code" })) as ApiErrorBody;
      setError(apiErrorMessage(body, "Unable to submit the authorization code"));
    } else {
      setCode("");
    }
  };

  return (
    <section className="harness-account-setup" aria-live="polite">
      <strong>Sign in to {harnessLabel} account “{account.name}”</strong>
      {!login ? <p>Starting the sign-in…</p> : null}
      {!isClaude && (login?.state === "queued" || login?.state === "authorizing") ? <p>Preparing the official sign-in flow…</p> : null}
      {isClaude && login?.state === "authorizing" ? <>
        <p>Sign in directly inside official Claude Code below. Paste any authentication code into that terminal, then click Done.</p>
        {sessionToken ? <div style={{ height: 420 }} data-activity-value="private">
          <TerminalView bindingId={login.id} sessionToken={sessionToken} reconnect={false} socketEndpoint={`/v1/ws/harness-logins/${encodeURIComponent(companyId)}/${encodeURIComponent(account.id)}/${encodeURIComponent(login.id)}`} />
        </div> : <p role="alert">Reopen account management to connect the sign-in terminal.</p>}
        <button type="button" className="btn-primary" disabled={checking || !sessionToken} onClick={() => void complete()}>{checking ? "Checking sign-in…" : "Done"}</button>
      </> : null}
      {!isClaude && login?.state === "awaiting_browser" ? (
          <>
            <p>Open the official Codex browser sign-in page.</p>
            {login.authorization_url ? <a className="btn-secondary" href={login.authorization_url} target="_blank" rel="noreferrer">Open sign-in link</a> : null}
            <p>This screen verifies the account automatically when the browser sign-in finishes. If you opened the link on another computer, paste its complete localhost callback URL below.</p>
            <label>
              Codex callback URL
              <input data-activity="codex_callback" data-activity-value="private" value={code} onChange={(event) => setCode(event.target.value)} autoComplete="off" />
              <button type="button" className="btn-secondary" disabled={!code.trim()} onClick={() => void submitCode()}>Finish sign-in</button>
            </label>
          </>
      ) : null}
      {login?.state === "failed" || login?.state === "expired" ? <p className="create-agent-warning" role="alert">{login.error || "Sign-in did not complete. Start it again."}</p> : null}
      {error ? <p className="create-agent-warning" role="alert">{error}</p> : null}
      <button type="button" className="btn-secondary" onClick={() => void cancel()}>Cancel</button>
    </section>
  );
}

/** Exact quota windows with a refresh that re-probes the account on the device that holds its login. */
function AccountUsage({ account, loading, onRefresh }: { account: HarnessAccount; loading: boolean; onRefresh: () => void }) {
  return (
    <div className="harness-account-usage" aria-label={`Exact usage for ${account.name}`}>
      {displayUsageWindows(account.driverType, account.usage.windows).map((window) => (
        <span key={window.id} title={window.resetsAt ? `Resets ${new Date(window.resetsAt).toLocaleString("en-US")}` : "Reset time unavailable"}>
          {window.label}: {window.remainingPercent}% left
        </span>
      ))}
      {account.probedAt ? <small>Updated {new Date(account.probedAt).toLocaleString("en-US")}</small> : null}
      {onRefresh ? (
        <button type="button" className="btn-secondary" disabled={loading} onClick={onRefresh}>
          {loading ? "Refreshing…" : "Refresh exact usage"}
        </button>
      ) : null}
    </div>
  );
}
