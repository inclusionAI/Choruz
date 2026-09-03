"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ConversationRuntimeStatus } from "../../lib/api/choruz-types";
import { apiFetch } from "../../lib/api/choruz-api";
import { Spinner } from "../ui/spinner";

function fetchConversationRuntimeStatusForClient(
  sessionToken: string,
  conversationId: string,
): Promise<ConversationRuntimeStatus[]> {
  return apiFetch<ConversationRuntimeStatus[]>(
    `/v1/conversations/${encodeURIComponent(conversationId)}/runtime-status`,
    sessionToken,
  );
}

function statusLabel(status: string): string {
  switch (status) {
    case "busy":
      return "busy";
    case "queued":
      return "queued";
    default:
      return "idle";
  }
}

function ageLabel(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds)) return "";
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

function queuedText(count: number): string {
  if (count === 1) return "1 queued turn";
  return `${count} queued turns`;
}

export function RuntimeStatusPanel({
  conversationId,
  sessionToken,
}: {
  conversationId: string;
  sessionToken: string;
}) {
  const [statuses, setStatuses] = useState<ConversationRuntimeStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval>>(undefined);

  const loadStatuses = useCallback(async () => {
    try {
      const data = await fetchConversationRuntimeStatusForClient(
        sessionToken,
        conversationId,
      );
      setStatuses(data);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load runtime status");
    } finally {
      setLoading(false);
    }
  }, [conversationId, sessionToken]);

  useEffect(() => {
    setLoading(true);
    loadStatuses();
    intervalRef.current = setInterval(loadStatuses, 8_000);
    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [loadStatuses]);

  const visibleStatuses = useMemo(() => {
    return [...statuses].sort((left, right) => {
      const priority = (status: string) =>
        status === "busy" ? 0 : status === "queued" ? 1 : 2;
      return (
        priority(left.status) - priority(right.status) ||
        right.queued_count - left.queued_count ||
        left.agent_name.localeCompare(right.agent_name)
      );
    });
  }, [statuses]);

  const queuedBanner = visibleStatuses.find((row) => row.queued_count > 0);

  return (
    <div className="detail-section">
      <div className="detail-section-header">
        <h4>Runtime queue</h4>
      </div>

      {queuedBanner && (
        <div className="runtime-queue-banner">
          {queuedBanner.agent_name} {queuedBanner.status === "busy" ? "is busy" : "has queued work"}.{" "}
          New messages will wait behind {queuedBanner.queued_count} earlier{" "}
          {queuedBanner.queued_count === 1 ? "turn" : "turns"}.
        </div>
      )}

      {loading ? (
        <p className="detail-inline-empty"><Spinner label="Loading…" /></p>
      ) : error ? (
        <p className="detail-inline-empty">{error}</p>
      ) : visibleStatuses.length === 0 ? (
        <p className="detail-inline-empty">No active agent runtimes</p>
      ) : (
        <div className="runtime-status-list">
          {visibleStatuses.map((row) => {
            const command = row.active_command;
            return (
              <div key={row.agent_principal_id} className="runtime-status-row">
                <div className="runtime-status-main">
                  <span className="runtime-status-agent">{row.agent_name}</span>
                  <span className={`runtime-status-pill runtime-status-${row.status}`}>
                    {statusLabel(row.status)}
                  </span>
                </div>
                <div className="runtime-status-meta">
                  <span>{queuedText(row.queued_count)}</span>
                  {command && (
                    <span>
                      active {ageLabel(command.lease_age_seconds)} - attempt{" "}
                      {command.attempt_count}
                    </span>
                  )}
                </div>
                {row.last_error && (
                  <div className="runtime-status-error">{row.last_error}</div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
