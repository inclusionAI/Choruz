"use client";

import { useEffect, useRef, useState } from "react";
import type { DashboardSyncChange } from "../lib/api/choruz-types";
import {
  loadDashboardSyncState,
  persistDashboardSyncCursor,
  type DashboardSyncState,
} from "../lib/messages/message-db";
import { trace } from "../lib/api/choruz-trace";
import { type DashboardSocket, transportSocket } from "../lib/api/transport";

export type ConnectionStatus = "connected" | "reconnecting";

type ServerFrame =
  | { type: "sync_ready"; device_id: string; cursor: number; head_cursor: number }
  | {
      type: "sync_changes";
      changes: DashboardSyncChange[];
      next_cursor: number;
      head_cursor: number;
      has_more: boolean;
    }
  | { type: "sync_acked"; cursor: number }
  | { type: "sync_error"; detail: string };

const RECONNECT_BASE_MS = 500;
const RECONNECT_MAX_MS = 16_000;

/** Authenticated, durable dashboard stream. A page is applied before its
 * cursor is persisted and ACKed, so a crash during apply safely replays it. */
export function useChatWebSocket(
  userId: string | null,
  bootstrapCursor: number,
  onChanges: (changes: DashboardSyncChange[]) => Promise<void> | void,
  onParseError?: (error: unknown) => void,
) {
  const [status, setStatus] = useState<ConnectionStatus>("connected");
  const onChangesRef = useRef(onChanges);
  onChangesRef.current = onChanges;
  const onParseErrorRef = useRef(onParseError);
  onParseErrorRef.current = onParseError;

  useEffect(() => {
    if (!userId) return;
    let stopped = false;
    let socket: DashboardSocket | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    let reconnectDelay = RECONNECT_BASE_MS;
    let syncState: DashboardSyncState | null = null;
    let applyChain = Promise.resolve();

    const scheduleReconnect = () => {
      if (stopped || reconnectTimer) return;
      setStatus("reconnecting");
      const delay = reconnectDelay;
      reconnectDelay = Math.min(reconnectDelay * 2, RECONNECT_MAX_MS);
      reconnectTimer = setTimeout(() => {
        reconnectTimer = null;
        void connect();
      }, delay);
    };

    const socketPath = (state: DashboardSyncState) => {
      const params = new URLSearchParams({
        device_id: state.device_id,
        cursor: String(state.ack_cursor),
      });
      return `/v1/ws/sync?${params}`;
    };

    const connect = async () => {
      if (stopped) return;
      syncState ??= await loadDashboardSyncState(userId, bootstrapCursor);
      if (stopped) return;
      const span = trace.start("dashboard_sync_connect", { cursor: syncState.ack_cursor });
      let sourceSocket: DashboardSocket;
      try {
        sourceSocket = transportSocket(socketPath(syncState));
        socket = sourceSocket;
      } catch (error) {
        span.end({ error: String(error) });
        scheduleReconnect();
        return;
      }

      sourceSocket.onopen = () => {
        if (stopped || socket !== sourceSocket) return;
        span.end({ status: "connected" });
        reconnectDelay = RECONNECT_BASE_MS;
        setStatus("connected");
      };
      sourceSocket.onmessage = (event) => {
        const applyChanges = onChangesRef.current;
        const reportParseError = onParseErrorRef.current;
        try {
          const frame = JSON.parse(String(event.data)) as ServerFrame;
          if (frame.type === "sync_changes") {
            // A browser can receive the next page while IndexedDB/state work
            // for the previous page is still pending. Serialize application
            // and ACKs so cursors can never overtake local state.
            applyChain = applyChain.then(async () => {
              if (stopped || socket !== sourceSocket) return;
              await applyChanges(frame.changes);
              if (stopped || socket !== sourceSocket) return;
              syncState = await persistDashboardSyncCursor(syncState!, frame.next_cursor);
              if (!stopped && socket === sourceSocket && sourceSocket.readyState === WebSocket.OPEN) {
                sourceSocket.send(JSON.stringify({ type: "sync_ack", cursor: frame.next_cursor }));
              }
            }).catch((error) => {
              trace.event("dashboard_sync_frame_error", { error: String(error) });
              reportParseError?.(error);
              if (socket === sourceSocket) sourceSocket.close();
            });
          } else if (frame.type === "sync_error") {
            throw new Error(frame.detail);
          }
        } catch (error) {
          trace.event("dashboard_sync_frame_error", { error: String(error) });
          reportParseError?.(error);
          if (socket === sourceSocket) sourceSocket.close();
        }
      };
      sourceSocket.onerror = () => sourceSocket.close();
      sourceSocket.onclose = () => {
        if (socket !== sourceSocket) return;
        socket = null;
        scheduleReconnect();
      };
    };

    void connect();
    return () => {
      stopped = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      socket?.close();
    };
  }, [bootstrapCursor, userId]);

  return { status };
}
