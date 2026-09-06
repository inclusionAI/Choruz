import Dexie, { type Table } from "dexie";
import type { ChoruzTransport } from "./transport";

export interface ActivityEvent {
  eventId: string;
  schemaVersion: 1;
  traceId: string;
  spanId: string;
  sessionId: string;
  name: string;
  ts: string;
  durationMs?: number;
  data?: Record<string, unknown>;
}

interface PendingActivity {
  eventId: string;
  scope: string;
  event: ActivityEvent;
}

/** A device-scoped durable queue. Credentials and transport handles never enter IndexedDB. */
export class ActivityOutbox {
  private db: Dexie;
  private pending: Table<PendingActivity, string>;
  private sending = false;

  constructor(databaseName = "choruz-activity") {
    this.db = new Dexie(databaseName);
    this.db.version(1).stores({ pending: "eventId,scope" });
    this.pending = this.db.table("pending");
  }

  async append(scope: string, event: ActivityEvent): Promise<void> {
    if (new TextEncoder().encode(JSON.stringify(event.data ?? {})).length > 16_384) {
      event = { ...event, data: { payload_omitted: "size_limit" } };
    }
    await this.pending.put({ eventId: event.eventId, scope, event });
  }

  /** Removes only an acknowledged batch. Concurrent tabs may replay IDs safely.
   * Callers retry failures with the same scope; never send another actor's queue. */
  async flush(scope: string, token: string, transport: ChoruzTransport): Promise<void> {
    if (this.sending) return;
    this.sending = true;
    try {
      const batch = await this.pending.where("scope").equals(scope).limit(100).toArray();
      if (!batch.length) return;
      const response = await transport.fetch("/api/v1/telemetry", {
        method: "POST",
        headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
        body: JSON.stringify({ events: batch.map(row => row.event) }),
        signal: AbortSignal.timeout(15_000),
      });
      if (response.status !== 204) throw new Error(`Activity delivery failed (${response.status})`);
      await this.pending.bulkDelete(batch.map(row => row.eventId));
    } finally {
      this.sending = false;
    }
  }

  close(): void {
    this.db.close();
  }
}
