"use client";

import { useEffect, useState } from "react";
import { apiFetch, ApiRequestError } from "../../lib/api/choruz-api";
import type { DashboardBootstrap } from "../../lib/api/choruz-types";
import { dashboardSnapshotFromBootstrap, type DashboardSnapshotProps } from "../../lib/api/dashboard-snapshot";
import { ChatApp } from "./chat-app";

export function Dashboard({ sessionToken }: { sessionToken: string }) {
  const [snapshot, setSnapshot] = useState<DashboardSnapshotProps | null>(null);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function connect() {
      try {
        const bootstrap = await apiFetch<DashboardBootstrap>("/v1/bootstrap?limit=100", sessionToken, { signal: controller.signal });
        if (!controller.signal.aborted) setSnapshot(dashboardSnapshotFromBootstrap(bootstrap));
      } catch (error) {
        if (controller.signal.aborted) return;
        if (error instanceof ApiRequestError && error.status === 401) {
          window.location.replace("/auth/session-invalid");
          return;
        }
        setFailed(true);
        timer = setTimeout(() => void connect(), 3000);
      }
    }
    setFailed(false);
    void connect();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [sessionToken, attempt]);

  if (snapshot) return <ChatApp sessionToken={sessionToken} {...snapshot} />;
  return <main className="dashboard-connection" aria-live="polite">
    <h1>{failed ? "Unable to connect to your workspace" : "Connecting to your workspace…"}</h1>
    {failed && <><p>Your tasks will appear when the connection recovers.</p>
      <button className="btn-primary" type="button" onClick={() => setAttempt((value) => value + 1)}>Retry connection</button></>}
  </main>;
}
