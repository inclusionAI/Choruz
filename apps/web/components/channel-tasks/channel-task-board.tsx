import {
  AlertCircle,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  Circle,
  CircleDot,
  Loader2,
} from "lucide-react";
import { useId } from "react";

import type { ChannelTask, ChannelTaskStatus, PatchChannelTaskRequest, Principal } from "../../lib/api/choruz-types";
import { EmptyState } from "../ui/empty-state";
import { Spinner } from "../ui/spinner";

export const CHANNEL_TASK_STATUS_OPTIONS: Array<{ status: ChannelTaskStatus; label: string }> = [
  { status: "todo", label: "Todo" },
  { status: "in_progress", label: "In Progress" },
  { status: "blocked", label: "Blocked" },
  { status: "in_review", label: "In Review" },
  { status: "done", label: "Done" },
];

const STATUS_ICONS: Record<ChannelTaskStatus, typeof Circle> = {
  todo: Circle,
  in_progress: CircleDot,
  blocked: AlertCircle,
  in_review: Loader2,
  done: CheckCircle2,
};

type ChannelTaskBoardProps = {
  tasks: ChannelTask[];
  visibleAssignees: Principal[];
  loading: boolean;
  error: string | null;
  mutatingTaskIds: Set<string>;
  onPatchTask: (taskId: string, patch: PatchChannelTaskRequest) => void;
};

export function ChannelTaskBoard({
  tasks,
  visibleAssignees,
  loading,
  error,
  mutatingTaskIds,
  onPatchTask,
}: ChannelTaskBoardProps) {
  const hasTasks = tasks.length > 0;

  if (loading && tasks.length === 0) {
    return (
      <div id="conversation-tasks-panel" role="tabpanel" aria-labelledby="conversation-tasks-tab" className="channel-task-board-state">
        <Spinner size={18} label="Loading tasks…" />
      </div>
    );
  }

  if (error && tasks.length === 0) {
    return <div id="conversation-tasks-panel" role="tabpanel" aria-labelledby="conversation-tasks-tab" className="channel-task-board-state error">{error}</div>;
  }

  return (
    <section id="conversation-tasks-panel" role="tabpanel" aria-labelledby="conversation-tasks-tab" className="channel-task-board">
      {!hasTasks ? (
        <EmptyState inline className="channel-task-board-empty" description="No tasks yet. Create one from a message to start the board." />
      ) : null}
      {CHANNEL_TASK_STATUS_OPTIONS.map((column) => {
        const columnTasks = tasks.filter((task) => task.status === column.status);
        return (
          <div className="channel-task-column" key={column.status}>
            <div className="channel-task-column-header">
              <span>{column.label}</span>
              <span>{columnTasks.length}</span>
            </div>
            <div className="channel-task-column-body">
              {columnTasks.length === 0 ? (
                <div className="channel-task-empty">Empty</div>
              ) : (
                columnTasks.map((task) => (
                  <ChannelTaskCard
                    key={task.task_id}
                    task={task}
                    visibleAssignees={visibleAssignees}
                    mutating={mutatingTaskIds.has(task.task_id)}
                    onPatchTask={onPatchTask}
                  />
                ))
              )}
            </div>
          </div>
        );
      })}
      {error ? <div className="channel-task-board-toast">{error}</div> : null}
    </section>
  );
}

type ChannelTaskCardProps = {
  task: ChannelTask;
  visibleAssignees: Principal[];
  mutating: boolean;
  onPatchTask: (taskId: string, patch: PatchChannelTaskRequest) => void;
};

function ChannelTaskCard({
  task,
  visibleAssignees,
  mutating,
  onPatchTask,
}: ChannelTaskCardProps) {
  const controlId = useId();
  const titleId = `${controlId}-title`;
  const statusSelectId = `${controlId}-status`;
  const statusDescriptionId = `${controlId}-status-description`;
  const assigneeSelectId = `${controlId}-assignee`;
  const StatusIcon = STATUS_ICONS[task.status];
  const previousStatus = adjacentChannelTaskStatus(task.status, "left");
  const nextStatus = adjacentChannelTaskStatus(task.status, "right");
  const currentStatusLabel = statusLabel(task.status);
  const previousStatusLabel = previousStatus ? statusLabel(previousStatus) : null;
  const nextStatusLabel = nextStatus ? statusLabel(nextStatus) : null;
  const assigneeLabel = task.assignee_name ?? "Unassigned";
  const updatedLabel = formatUpdatedAt(task.updated_at);

  const patchStatus = (status: ChannelTaskStatus) => {
    patchChannelTaskStatus(task.task_id, task.status, status, onPatchTask);
  };

  return (
    <article className="channel-task-card" aria-busy={mutating} aria-labelledby={titleId}>
      <div className="channel-task-card-topline">
        <span className="channel-task-key">{task.task_key}</span>
        <span
          className="channel-task-status-mark"
          title={currentStatusLabel}
          aria-label={`Current status: ${currentStatusLabel}`}
        >
          <StatusIcon size={14} />
        </span>
      </div>
      <h3 id={titleId}>{task.title}</h3>
      <div className="channel-task-meta">
        <span>{assigneeLabel}</span>
        {updatedLabel ? <span>{updatedLabel}</span> : null}
      </div>
      {task.context_label ? (
        <div className="channel-task-context">{task.context_label}</div>
      ) : null}
      {task.blocked_reason ? (
        <div className="channel-task-blocked">{task.blocked_reason}</div>
      ) : null}
      <div className="channel-task-controls">
        <div className="channel-task-control">
          <div className="channel-task-control-heading">
            <label className="channel-task-control-label" htmlFor={statusSelectId}>Status</label>
            <span id={statusDescriptionId} className="channel-task-status-current">
              Current: {currentStatusLabel}
            </span>
          </div>
          <div className="channel-task-status-controls">
            <button
              type="button"
              aria-label={
                previousStatusLabel
                  ? `Move ${task.task_key} to ${previousStatusLabel}`
                  : `${task.task_key} is already in the first status`
              }
              title={previousStatusLabel ? `Move to ${previousStatusLabel}` : "First status"}
              disabled={mutating || !previousStatus}
              onClick={() => previousStatus && patchStatus(previousStatus)}
            >
              <ChevronLeft size={14} />
            </button>
            <select
              id={statusSelectId}
              aria-label={`Move ${task.task_key} status`}
              aria-describedby={statusDescriptionId}
              value={task.status}
              disabled={mutating}
              onChange={(event) =>
                handleChannelTaskStatusSelectChange(
                  task.task_id,
                  task.status,
                  event.target.value as ChannelTaskStatus,
                  onPatchTask,
                )
              }
            >
              {CHANNEL_TASK_STATUS_OPTIONS.map((column) => (
                <option key={column.status} value={column.status}>
                  {column.label}
                </option>
              ))}
            </select>
            <button
              type="button"
              aria-label={
                nextStatusLabel
                  ? `Move ${task.task_key} to ${nextStatusLabel}`
                  : `${task.task_key} is already in the final status`
              }
              title={nextStatusLabel ? `Move to ${nextStatusLabel}` : "Final status"}
              disabled={mutating || !nextStatus}
              onClick={() => nextStatus && patchStatus(nextStatus)}
            >
              <ChevronRight size={14} />
            </button>
          </div>
        </div>
        <div className="channel-task-control">
          <label className="channel-task-control-label" htmlFor={assigneeSelectId}>Assignee</label>
          <select
            id={assigneeSelectId}
            value={
              visibleAssignees.some((assignee) => assignee.id === task.assignee_principal_id)
                ? task.assignee_principal_id
                : ""
            }
            disabled={mutating || visibleAssignees.length === 0}
            onChange={(event) =>
              handleChannelTaskAssigneeSelectChange(
                task.task_id,
                task.assignee_principal_id,
                event.target.value,
                onPatchTask,
              )
            }
          >
            <option value="" disabled>
              Select assignee
            </option>
            {visibleAssignees.map((assignee) => (
              <option key={assignee.id} value={assignee.id}>
                {assignee.name}
              </option>
            ))}
          </select>
        </div>
      </div>
    </article>
  );
}

function statusLabel(status: ChannelTaskStatus): string {
  return CHANNEL_TASK_STATUS_OPTIONS.find((column) => column.status === status)?.label ?? status;
}

export function channelTaskStatusPatch(
  currentStatus: ChannelTaskStatus,
  selectedStatus: ChannelTaskStatus,
): PatchChannelTaskRequest | null {
  return selectedStatus === currentStatus ? null : { status: selectedStatus };
}

export function patchChannelTaskStatus(
  taskId: string,
  currentStatus: ChannelTaskStatus,
  selectedStatus: ChannelTaskStatus,
  onPatchTask: (taskId: string, patch: PatchChannelTaskRequest) => void,
) {
  const patch = channelTaskStatusPatch(currentStatus, selectedStatus);
  if (patch) {
    onPatchTask(taskId, patch);
  }
}

export function handleChannelTaskStatusSelectChange(
  taskId: string,
  currentStatus: ChannelTaskStatus,
  selectedStatus: ChannelTaskStatus,
  onPatchTask: (taskId: string, patch: PatchChannelTaskRequest) => void,
) {
  patchChannelTaskStatus(taskId, currentStatus, selectedStatus, onPatchTask);
}

export function channelTaskAssigneePatch(
  currentAssigneePrincipalId: string | undefined,
  selectedAssigneePrincipalId: string,
): PatchChannelTaskRequest | null {
  if (!selectedAssigneePrincipalId || selectedAssigneePrincipalId === currentAssigneePrincipalId) {
    return null;
  }
  return { assignee_principal_id: selectedAssigneePrincipalId };
}

export function patchChannelTaskAssignee(
  taskId: string,
  currentAssigneePrincipalId: string | undefined,
  selectedAssigneePrincipalId: string,
  onPatchTask: (taskId: string, patch: PatchChannelTaskRequest) => void,
) {
  const patch = channelTaskAssigneePatch(currentAssigneePrincipalId, selectedAssigneePrincipalId);
  if (patch) {
    onPatchTask(taskId, patch);
  }
}

export function handleChannelTaskAssigneeSelectChange(
  taskId: string,
  currentAssigneePrincipalId: string | undefined,
  selectedAssigneePrincipalId: string,
  onPatchTask: (taskId: string, patch: PatchChannelTaskRequest) => void,
) {
  patchChannelTaskAssignee(taskId, currentAssigneePrincipalId, selectedAssigneePrincipalId, onPatchTask);
}

export function adjacentChannelTaskStatus(
  status: ChannelTaskStatus,
  direction: "left" | "right",
): ChannelTaskStatus | null {
  const statusIndex = CHANNEL_TASK_STATUS_OPTIONS.findIndex((column) => column.status === status);
  const nextIndex = direction === "left" ? statusIndex - 1 : statusIndex + 1;
  return CHANNEL_TASK_STATUS_OPTIONS[nextIndex]?.status ?? null;
}

function formatUpdatedAt(value: string): string | null {
  const timestamp = Date.parse(value);
  if (Number.isNaN(timestamp)) return null;
  return new Intl.DateTimeFormat("en", {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(timestamp));
}
