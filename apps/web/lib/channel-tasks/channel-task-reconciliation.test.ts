import { describe, expect, it } from "vitest";

import {
  mergeChannelTaskList,
  replaceChannelTaskList,
  replaceChannelTask,
  applyChannelTaskCreateResponse,
  restoreChannelTaskAfterFailedMutation,
} from "./channel-task-reconciliation";
import type { ChannelTask } from "../api/choruz-types";

describe("channel task reconciliation", () => {
  it("keeps a newer local task when a stale refresh completes later", () => {
    const current = task({
      task_id: "task-1",
      status: "done",
      version: 3,
      updated_at: "2026-05-01T03:00:00Z",
    });
    const stale = task({
      task_id: "task-1",
      status: "todo",
      version: 2,
      updated_at: "2026-05-01T02:00:00Z",
    });

    expect(mergeChannelTaskList([current], [stale])[0]).toMatchObject({
      status: "done",
      version: 3,
    });
  });

  it("accepts a newer server task and appends unseen tasks", () => {
    const current = task({ task_id: "task-1", version: 1 });
    const updated = task({ task_id: "task-1", status: "blocked", version: 2 });
    const unseen = task({ task_id: "task-2", task_key: "TASK-2", version: 1 });

    const merged = mergeChannelTaskList([current], [updated, unseen]);

    expect(merged).toHaveLength(2);
    expect(merged.find((item) => item.task_id === "task-1")).toMatchObject({
      status: "blocked",
      version: 2,
    });
  });

  it("does not rollback over a newer task after a mutation failure", () => {
    const previous = task({ task_id: "task-1", status: "todo", version: 1 });
    const newer = task({ task_id: "task-1", status: "done", version: 2 });

    expect(restoreChannelTaskAfterFailedMutation([newer], previous)[0]).toMatchObject({
      status: "done",
      version: 2,
    });
  });

  it("rolls back a failed same-version optimistic mutation even when its client timestamp is newer", () => {
    const previous = task({
      task_id: "task-1",
      status: "todo",
      version: 1,
      updated_at: "2026-05-01T00:00:00Z",
    });
    const failedOptimistic = task({
      task_id: "task-1",
      status: "blocked",
      version: 1,
      updated_at: "2026-05-01T00:00:01Z",
    });

    expect(restoreChannelTaskAfterFailedMutation([failedOptimistic], previous)[0]).toMatchObject({
      status: "todo",
      updated_at: "2026-05-01T00:00:00Z",
    });
  });

  it("replaces or appends a patched task deterministically", () => {
    const current = task({ task_id: "task-1", status: "todo" });
    const patched = task({ task_id: "task-1", status: "in_review", version: 2 });
    const appended = task({ task_id: "task-2", task_key: "TASK-2" });

    expect(replaceChannelTask([current], patched)).toEqual([patched]);
    expect(replaceChannelTask([current], appended).map((item) => item.task_id).sort()).toEqual([
      "task-1",
      "task-2",
    ]);
  });

  it("does not replace a newer task with a stale mutation response", () => {
    const newer = task({
      task_id: "task-1",
      status: "done",
      version: 5,
      updated_at: "2026-05-01T05:00:00Z",
    });
    const stalePatchResponse = task({
      task_id: "task-1",
      status: "blocked",
      version: 4,
      updated_at: "2026-05-01T04:00:00Z",
    });

    expect(replaceChannelTask([newer], stalePatchResponse)[0]).toMatchObject({
      status: "done",
      version: 5,
    });
  });

  it("keeps an optimistic equal-version task when a stale refresh has the same timestamp", () => {
    const optimistic = task({
      task_id: "task-1",
      status: "blocked",
      version: 1,
      updated_at: "2026-05-01T00:00:01Z",
    });
    const staleRefresh = task({
      task_id: "task-1",
      status: "todo",
      version: 1,
      updated_at: "2026-05-01T00:00:00Z",
    });

    expect(mergeChannelTaskList([optimistic], [staleRefresh])[0]).toMatchObject({
      status: "blocked",
      updated_at: "2026-05-01T00:00:01Z",
    });
  });

  it("replaces local state when a forced refetch returns the server snapshot", () => {
    const newerLocal = task({
      task_id: "task-1",
      status: "done",
      version: 5,
      updated_at: "2026-05-01T05:00:00Z",
    });
    const serverSnapshot = task({
      task_id: "task-1",
      status: "blocked",
      version: 4,
      updated_at: "2026-05-01T04:00:00Z",
    });

    expect(mergeChannelTaskList([newerLocal], [serverSnapshot])[0]).toMatchObject({
      status: "done",
      version: 5,
    });
    expect(replaceChannelTaskList([serverSnapshot])).toEqual([serverSnapshot]);
  });

  it("uses the create-from-message response as server truth for deduped tasks", () => {
    const localDraftMutation = task({
      task_id: "task-1",
      title: "Changed title from a repeated create attempt",
      assignee_principal_id: "agent-2",
      assignee_name: "Grace",
      version: 1,
      updated_at: "2026-05-01T00:00:10Z",
    });
    const existingServerTask = task({
      task_id: "task-1",
      title: "Original message task",
      assignee_principal_id: "agent-1",
      assignee_name: "Ada",
      version: 1,
      updated_at: "2026-05-01T00:00:00Z",
    });

    expect(applyChannelTaskCreateResponse([localDraftMutation], existingServerTask)[0]).toMatchObject({
      title: "Original message task",
      assignee_principal_id: "agent-1",
      assignee_name: "Ada",
    });
  });

  it("does not let a stale create-from-message response overwrite newer local task state", () => {
    const newerLocal = task({
      task_id: "task-1",
      title: "Original message task",
      status: "done",
      version: 3,
      updated_at: "2026-05-01T00:00:30Z",
    });
    const staleCreateResponse = task({
      task_id: "task-1",
      title: "Original message task",
      status: "todo",
      version: 2,
      updated_at: "2026-05-01T00:00:20Z",
    });

    expect(applyChannelTaskCreateResponse([newerLocal], staleCreateResponse)[0]).toMatchObject({
      status: "done",
      version: 3,
    });
  });
});

function task(overrides: Partial<ChannelTask>): ChannelTask {
  return {
    task_id: "task-1",
    conversation_id: "conv-1",
    task_key: "TASK-1",
    title: "Follow up",
    status: "todo",
    assignee_principal_id: "human-1",
    assignee_type: "human",
    assignee_name: "Pat",
    source_kind: "message",
    source_message_id: "message-1",
    created_by: "human-1",
    created_by_type: "human",
    updated_by: "human-1",
    updated_by_type: "human",
    version: 1,
    created_at: "2026-05-01T00:00:00Z",
    updated_at: "2026-05-01T00:00:00Z",
    ...overrides,
  };
}
