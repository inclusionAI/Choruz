-- 0025: Channel Kanban board persistence foundation.
--
-- The pre-MVP workflow-task surface is not released. Per the channel Kanban
-- RFC, legacy rows are hard-reset so the board can tighten status, ownership,
-- idempotency, and redaction invariants without ambiguous backfill semantics.

BEGIN;

DELETE FROM group_workflow_event;
DELETE FROM group_workflow_task_participant;
DELETE FROM group_workflow_task;

ALTER TABLE principal
  ADD COLUMN IF NOT EXISTS channel_visibility TEXT NOT NULL DEFAULT 'visible';

DO $$ BEGIN
  ALTER TABLE principal
    ADD CONSTRAINT principal_channel_visibility_check
      CHECK (channel_visibility IN ('visible', 'internal'));
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

CREATE INDEX IF NOT EXISTS idx_principal_channel_visible_agents
  ON principal (workspace_id, channel_visibility)
  WHERE type = 'agent' AND disabled = FALSE AND deleted_at IS NULL;

ALTER TABLE group_workflow_task
  DROP CONSTRAINT IF EXISTS group_workflow_task_status_check;

ALTER TABLE group_workflow_task
  ADD COLUMN IF NOT EXISTS assignee_principal_id TEXT REFERENCES principal(id) ON DELETE RESTRICT,
  ADD COLUMN IF NOT EXISTS blocked_reason TEXT,
  ADD COLUMN IF NOT EXISTS source_kind TEXT NOT NULL DEFAULT 'agent',
  ADD COLUMN IF NOT EXISTS context_label TEXT,
  ADD COLUMN IF NOT EXISTS idempotency_key TEXT,
  ADD COLUMN IF NOT EXISTS idempotency_payload_hash TEXT,
  ADD COLUMN IF NOT EXISTS version BIGINT NOT NULL DEFAULT 1;

ALTER TABLE group_workflow_task
  DROP COLUMN IF EXISTS description;

ALTER TABLE group_workflow_task
  ALTER COLUMN assignee_principal_id SET NOT NULL;

DO $$ BEGIN
  ALTER TABLE group_workflow_task
    ADD CONSTRAINT group_workflow_task_status_check
      CHECK (status IN ('todo', 'in_progress', 'blocked', 'in_review', 'done'));
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE group_workflow_task
    ADD CONSTRAINT group_workflow_task_source_kind_check
      CHECK (source_kind IN ('agent', 'message'));
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE group_workflow_task
    ADD CONSTRAINT group_workflow_task_message_source_check
      CHECK (source_kind <> 'message' OR source_message_id IS NOT NULL);
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

ALTER TABLE group_workflow_task
  ALTER COLUMN status SET DEFAULT 'todo';

CREATE INDEX IF NOT EXISTS idx_group_workflow_task_assignee
  ON group_workflow_task (assignee_principal_id);

CREATE INDEX IF NOT EXISTS idx_group_workflow_task_conversation_assignee
  ON group_workflow_task (conversation_id, assignee_principal_id);

-- Routing compatibility is derived from the canonical board assignee. The
-- service keeps exactly one owner participant synchronized for each task.
CREATE UNIQUE INDEX IF NOT EXISTS group_workflow_task_single_owner_participant_idx
  ON group_workflow_task_participant (task_id)
  WHERE role_key = 'owner';

CREATE UNIQUE INDEX IF NOT EXISTS group_workflow_task_agent_idempotency_idx
  ON group_workflow_task (conversation_id, created_by, idempotency_key)
  WHERE idempotency_key IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS group_workflow_task_message_dedupe_idx
  ON group_workflow_task (source_message_id, created_by)
  WHERE source_kind = 'message' AND source_message_id IS NOT NULL AND created_by IS NOT NULL;

CREATE TABLE IF NOT EXISTS channel_task_sequence (
  conversation_id TEXT PRIMARY KEY REFERENCES conversation(id) ON DELETE CASCADE,
  next_value BIGINT NOT NULL DEFAULT 1 CHECK (next_value > 0)
);

ALTER TABLE group_workflow_event
  ADD COLUMN IF NOT EXISTS resulting_version BIGINT;

COMMIT;
