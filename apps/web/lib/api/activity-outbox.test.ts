import "fake-indexeddb/auto";
import Dexie from "dexie";
import { afterEach, expect, it, vi } from "vitest";
import { ActivityOutbox, type ActivityEvent } from "./activity-outbox";
import { localTransport } from "./transport";

const databases: string[] = [];
afterEach(async () => { for (const name of databases.splice(0)) await Dexie.delete(name); });

function event(): ActivityEvent {
  return { eventId: crypto.randomUUID(), schemaVersion: 1, traceId: "trace", spanId: "span", sessionId: "session", name: "send_message", ts: new Date().toISOString() };
}

it("keeps an oversized event's identity without poisoning subsequent batches", async () => {
  const name = crypto.randomUUID(); databases.push(name);
  const outbox = new ActivityOutbox(name);
  const record = { ...event(), data: { message: "字".repeat(6000) } };
  const fetch = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
  try {
    await outbox.append("actor", record);
    await outbox.flush("actor", "token", { ...localTransport, fetch });
    const saved = JSON.parse(fetch.mock.calls[0][1].body).events[0];
    expect(saved.eventId).toBe(record.eventId);
    expect(saved.data).toEqual({ payload_omitted: "size_limit" });
  } finally { outbox.close(); }
});

it("retains events across HTTP failure and reopening, and deletes only after commit acknowledgement", async () => {
  const name = crypto.randomUUID(); databases.push(name);
  const first = new ActivityOutbox(name);
  const record = event();
  await first.append("actor-a", record);
  const fetch = vi.fn().mockResolvedValueOnce(new Response(null, { status: 500 })).mockResolvedValue(new Response(null, { status: 204 }));
  const transport = { ...localTransport, fetch };
  await expect(first.flush("actor-a", "ephemeral-token", transport)).rejects.toThrow("500");
  first.close();
  const reopened = new ActivityOutbox(name);
  try {
    await reopened.flush("actor-b", "other-token", transport);
    expect(fetch).toHaveBeenCalledTimes(1);
    await reopened.flush("actor-a", "ephemeral-token", transport);
    expect(JSON.parse(fetch.mock.calls[1][1].body).events).toEqual([record]);
    await reopened.flush("actor-a", "ephemeral-token", transport);
    expect(fetch).toHaveBeenCalledTimes(2);
  } finally { reopened.close(); }
});

it("keeps new events appended during a pending acknowledgement", async () => {
  const name = crypto.randomUUID(); databases.push(name);
  const outbox = new ActivityOutbox(name);
  let acknowledge!: (response: Response) => void;
  let started!: () => void;
  const barrier = new Promise<void>(resolve => { started = resolve; });
  const fetch = vi.fn().mockImplementationOnce(() => { started(); return new Promise<Response>(resolve => { acknowledge = resolve; }); }).mockResolvedValue(new Response(null, { status: 204 }));
  const transport = { ...localTransport, fetch };
  try {
    await outbox.append("actor", event());
    const sending = outbox.flush("actor", "token", transport);
    await barrier;
    const later = event();
    await outbox.append("actor", later);
    acknowledge(new Response(null, { status: 204 }));
    await sending;
    await outbox.flush("actor", "token", transport);
    expect(JSON.parse(fetch.mock.calls[1][1].body).events).toEqual([later]);
  } finally { outbox.close(); }
});
