import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import {
  adjacentChannelTaskStatus,
  channelTaskStatusPatch,
  CHANNEL_TASK_STATUS_OPTIONS,
  ChannelTaskBoard,
  channelTaskAssigneePatch,
  handleChannelTaskAssigneeSelectChange,
  handleChannelTaskStatusSelectChange,
  patchChannelTaskStatus,
  patchChannelTaskAssignee,
} from "./channel-task-board";
import type { ChannelTask, Principal } from "../../lib/api/choruz-types";

describe("ChannelTaskBoard", () => {
  it("renders fixed board columns and a quiet empty state", () => {
    const html = renderToStaticMarkup(
      createElement(ChannelTaskBoard, {
        tasks: [],
        visibleAssignees: [principal("human-1", "Pat", "human")],
        loading: false,
        error: null,
        mutatingTaskIds: new Set<string>(),
        onPatchTask: () => {},
      }),
    );

    expect(html).toContain("Todo");
    expect(html).toContain("In Progress");
    expect(html).toContain("Blocked");
    expect(html).toContain("In Review");
    expect(html).toContain("Done");
    expect(html).toContain("No tasks yet.");
  });

  it("renders card metadata, blocked reason, context label, and visible assignee options", () => {
    const html = renderToStaticMarkup(
      createElement(ChannelTaskBoard, {
        tasks: [
          task({
            task_id: "task-1",
            task_key: "TASK-1",
            title: "Ship the board shell",
            status: "blocked",
            assignee_principal_id: "agent-1",
            assignee_name: "Ada",
            assignee_type: "agent",
            blocked_reason: "Waiting for API review",
            context_label: "Launch",
          }),
        ],
        visibleAssignees: [
          principal("human-1", "Pat", "human"),
          principal("agent-1", "Ada", "agent"),
        ],
        loading: false,
        error: null,
        mutatingTaskIds: new Set<string>(),
        onPatchTask: () => {},
      }),
    );

    expect(html).toContain("TASK-1");
    expect(html).toContain("Ship the board shell");
    expect(html).toContain("Waiting for API review");
    expect(html).toContain("Launch");
    expect(html).toContain("Ada");
    expect(html).toContain("Current: Blocked");
    expect(html).toContain('aria-label="Current status: Blocked"');
    expect(html).toContain('aria-label="Move TASK-1 status"');
    expect(html).toContain("Move TASK-1 to In Progress");
    expect(html).toContain("Move TASK-1 to In Review");
    expect(html).toMatch(/<option value="blocked"(?: selected="")?>Blocked<\/option>/);
  });

  it("shows a non-visible current assignee on the card without adding it to picker options", () => {
    const html = renderToStaticMarkup(
      createElement(ChannelTaskBoard, {
        tasks: [
          task({
            task_id: "task-2",
            task_key: "TASK-2",
            assignee_principal_id: "missing-human",
            assignee_name: "Casey",
          }),
        ],
        visibleAssignees: [principal("agent-1", "Ada", "agent")],
        loading: false,
        error: null,
        mutatingTaskIds: new Set<string>(),
        onPatchTask: () => {},
      }),
    );

    expect(html).toContain("Casey");
    expect(html).not.toContain('value="missing-human"');
    expect(html).toContain("Select assignee");
  });

  it("renders load errors when the board has no tasks", () => {
    const html = renderToStaticMarkup(
      createElement(ChannelTaskBoard, {
        tasks: [],
        visibleAssignees: [principal("human-1", "Pat", "human")],
        loading: false,
        error: "Unable to load tasks",
        mutatingTaskIds: new Set<string>(),
        onPatchTask: () => {},
      }),
    );

    expect(html).toContain("Unable to load tasks");
    expect(html).toContain('class="channel-task-board-state error"');
  });

  it("renders mutation errors without hiding existing cards", () => {
    const html = renderToStaticMarkup(
      createElement(ChannelTaskBoard, {
        tasks: [task({ task_id: "task-1", task_key: "TASK-1", title: "Keep visible" })],
        visibleAssignees: [principal("human-1", "Pat", "human")],
        loading: false,
        error: "Unable to update task",
        mutatingTaskIds: new Set<string>(),
        onPatchTask: () => {},
      }),
    );

    expect(html).toContain("Keep visible");
    expect(html).toContain("Unable to update task");
    expect(html).toContain('class="channel-task-board-toast"');
  });

  it("marks mutating cards busy and disables status and assignee controls", () => {
    const html = renderToStaticMarkup(
      createElement(ChannelTaskBoard, {
        tasks: [task({ task_id: "task-1", task_key: "TASK-1" })],
        visibleAssignees: [principal("human-1", "Pat", "human")],
        loading: false,
        error: null,
        mutatingTaskIds: new Set<string>(["task-1"]),
        onPatchTask: () => {},
      }),
    );

    expect(html).toContain('aria-busy="true"');
    expect(html).toContain('aria-label="Move TASK-1 status"');
    expect(html).toContain("disabled");
  });

  it("maps adjacent status movement without wrapping past fixed board edges", () => {
    expect(adjacentChannelTaskStatus("todo", "left")).toBeNull();
    expect(adjacentChannelTaskStatus("todo", "right")).toBe("in_progress");
    expect(adjacentChannelTaskStatus("blocked", "left")).toBe("in_progress");
    expect(adjacentChannelTaskStatus("blocked", "right")).toBe("in_review");
    expect(adjacentChannelTaskStatus("done", "right")).toBeNull();
  });

  it("maps status menu selections to existing task patch payloads", () => {
    expect(CHANNEL_TASK_STATUS_OPTIONS.map((option) => option.status)).toEqual([
      "todo",
      "in_progress",
      "blocked",
      "in_review",
      "done",
    ]);
    expect(channelTaskStatusPatch("todo", "blocked")).toEqual({ status: "blocked" });
    expect(channelTaskStatusPatch("blocked", "blocked")).toBeNull();
  });

  it("calls the existing patch callback when status controls choose a new status", () => {
    const onPatchTask = vi.fn();

    patchChannelTaskStatus("task-1", "todo", "in_progress", onPatchTask);
    patchChannelTaskStatus("task-1", "in_progress", "in_progress", onPatchTask);
    handleChannelTaskStatusSelectChange("task-2", "todo", "blocked", onPatchTask);

    expect(onPatchTask).toHaveBeenCalledTimes(2);
    expect(onPatchTask).toHaveBeenNthCalledWith(1, "task-1", { status: "in_progress" });
    expect(onPatchTask).toHaveBeenNthCalledWith(2, "task-2", { status: "blocked" });
  });

  it("maps assignee picker selections to existing task patch payloads", () => {
    expect(channelTaskAssigneePatch("human-1", "agent-1")).toEqual({
      assignee_principal_id: "agent-1",
    });
    expect(channelTaskAssigneePatch("human-1", "human-1")).toBeNull();
    expect(channelTaskAssigneePatch("human-1", "")).toBeNull();
  });

  it("calls the existing patch callback when assignee controls choose a new assignee", () => {
    const onPatchTask = vi.fn();

    patchChannelTaskAssignee("task-1", "human-1", "agent-1", onPatchTask);
    patchChannelTaskAssignee("task-1", "agent-1", "agent-1", onPatchTask);
    handleChannelTaskAssigneeSelectChange("task-2", "human-1", "agent-2", onPatchTask);

    expect(onPatchTask).toHaveBeenCalledTimes(2);
    expect(onPatchTask).toHaveBeenNthCalledWith(1, "task-1", { assignee_principal_id: "agent-1" });
    expect(onPatchTask).toHaveBeenNthCalledWith(2, "task-2", { assignee_principal_id: "agent-2" });
  });
});

function principal(
  id: string,
  name: string,
  principalType: Principal["principal_type"],
): Principal {
  return {
    id,
    workspace_id: "workspace-1",
    principal_type: principalType,
    name,
    avatar_url: null,
    scopes: [],
    disabled: false,
    created_at: "2026-05-01T00:00:00Z",
    updated_at: "2026-05-01T00:00:00Z",
  };
}

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
