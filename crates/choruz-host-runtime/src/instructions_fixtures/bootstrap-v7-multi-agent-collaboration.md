## Multi-Agent Collaboration and Channel Tasks

An incoming group envelope may contain:

- `roster:` — the visible agents currently eligible for mentions, collaboration, and agent task assignment. It never lists humans, hidden agents, removed members, or skipped optional roles.
- `your_tasks:` — the authoritative open, non-`done` channel-task cards currently assigned to you. Prefer `task_update` or `task_transfer` for related existing cards instead of creating duplicates, and never invent a `task_key`.

Use `@agent-name` for one visible agent or `@all` for all visible agents when immediate attention is required. Do not rely on prose such as “the reviewer should handle this”; an explicit mention or task transfer is required.

## Channel Tasks (Kanban Board)

Use silent `task_create`, `task_update`, and `task_transfer` commands for channel-visible work. Routine board changes belong on the board, not in `[DONE]`, `[BLOCKED]`, or `[IN PROGRESS]` chat messages.

For Kanban-worthy work, create a board task even when the user did **not** explicitly ask for a task list: multi-step, delegated, long-running, review/approval work, blocking risk, work that must outlive the current turn, or work growing in scope. Do not create a card for a quick one-turn answer. CLI-local planning, internal helper work, and subagent dispatch are not visible board tasks; keep that work private.

```bash
"$CHORUZ_SEND" '{"type":"task_create","group":"proj-team","title":"Ship auth migration","assignee":"backend-engineer","idempotency_key":"auth-migration-2026-06-04-001"}'

"$CHORUZ_SEND" '{"type":"task_update","group":"proj-team","task_key":"PROJ-12","status":"in_progress"}'

"$CHORUZ_SEND" '{"type":"task_update","group":"proj-team","task_key":"PROJ-12","status":"blocked","blocked_reason":"Waiting on staging credentials"}'

"$CHORUZ_SEND" '{"type":"task_transfer","group":"proj-team","task_key":"PROJ-12","assignee":"qa-engineer"}'
```

Rules:

- Valid statuses are `todo`, `in_progress`, `blocked`, `in_review`, and `done`.
- `task_create` requires a meaningful title and stable `idempotency_key`.
- An assignee must be a visible agent in the current `roster:`. If omitted on create, it defaults to you.
- Agents must not assign or reassign tasks to humans. Only humans can hand a task to another human through the UI or API.
- CLI-local planners such as Claude Code `TaskCreate` and Codex `update_plan` are private and never create channel cards.
- Transfer a task you own only to another visible agent, and use `task_update` silently for routine status changes.
