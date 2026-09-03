import type { ChannelTask } from "../api/choruz-types";

export function mergeChannelTaskList(existing: ChannelTask[], incoming: ChannelTask[]): ChannelTask[] {
  const byId = new Map<string, ChannelTask>();
  for (const task of existing) {
    byId.set(task.task_id, task);
  }
  for (const task of incoming) {
    const current = byId.get(task.task_id);
    byId.set(task.task_id, current && isChannelTaskNewer(current, task) ? current : task);
  }
  return [...byId.values()].sort(compareChannelTasksForBoard);
}

export function replaceChannelTaskList(incoming: ChannelTask[]): ChannelTask[] {
  return [...incoming].sort(compareChannelTasksForBoard);
}

export function replaceChannelTask(tasks: ChannelTask[], updatedTask: ChannelTask): ChannelTask[] {
  let replaced = false;
  const next = tasks.map((task) => {
    if (task.task_id !== updatedTask.task_id) return task;
    replaced = true;
    return isChannelTaskNewer(task, updatedTask) ? task : updatedTask;
  });
  return (replaced ? next : [...next, updatedTask]).sort(compareChannelTasksForBoard);
}

export function applyChannelTaskCreateResponse(tasks: ChannelTask[], createdTask: ChannelTask): ChannelTask[] {
  let replaced = false;
  const next = tasks.map((task) => {
    if (task.task_id !== createdTask.task_id) return task;
    replaced = true;
    if (task.version > createdTask.version) return task;
    return createdTask;
  });
  return (replaced ? next : [...next, createdTask]).sort(compareChannelTasksForBoard);
}

export function restoreChannelTaskAfterFailedMutation(
  tasks: ChannelTask[],
  previousTask: ChannelTask,
): ChannelTask[] {
  return tasks.map((task) => {
    if (task.task_id !== previousTask.task_id) return task;
    return task.version > previousTask.version ? task : previousTask;
  }).sort(compareChannelTasksForBoard);
}

export function isChannelTaskNewer(candidate: ChannelTask, current: ChannelTask): boolean {
  if (candidate.version !== current.version) {
    return candidate.version > current.version;
  }
  return Date.parse(candidate.updated_at) > Date.parse(current.updated_at);
}

function compareChannelTasksForBoard(left: ChannelTask, right: ChannelTask): number {
  const leftTime = Date.parse(left.updated_at);
  const rightTime = Date.parse(right.updated_at);
  if (leftTime !== rightTime) return rightTime - leftTime;
  return left.task_key.localeCompare(right.task_key);
}
