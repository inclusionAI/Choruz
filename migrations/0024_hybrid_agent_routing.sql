-- 0024: Hybrid agent routing policy and shared group workflow task state.

BEGIN;

ALTER TABLE conversation_runtime_policies
  ADD COLUMN IF NOT EXISTS default_coordinator_agent_id TEXT;

ALTER TABLE conversation_runtime_policies
  ADD COLUMN IF NOT EXISTS untagged_human_mode TEXT NOT NULL DEFAULT 'mentioned_only';

DO $$ BEGIN
  ALTER TABLE conversation_runtime_policies
    ADD CONSTRAINT conversation_runtime_policies_untagged_human_mode_check
      CHECK (untagged_human_mode IN ('mentioned_only', 'coordinator_only', 'all_agents'));
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE conversation_runtime_policies
    ADD CONSTRAINT fk_crp_default_coordinator_agent
      FOREIGN KEY (default_coordinator_agent_id) REFERENCES principal(id) ON DELETE SET NULL;
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

CREATE INDEX IF NOT EXISTS conversation_runtime_policies_default_coordinator_idx
  ON conversation_runtime_policies (default_coordinator_agent_id);

CREATE TABLE IF NOT EXISTS group_workflow_task (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversation(id) ON DELETE CASCADE,
    task_key TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (
        status IN (
            'pending',
            'in_progress',
            'blocked',
            'needs_human',
            'needs_approval',
            'completed'
        )
    ),
    source_message_id TEXT,
    created_by TEXT REFERENCES principal(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (conversation_id, task_key)
);

CREATE INDEX IF NOT EXISTS idx_group_workflow_task_conversation_status
  ON group_workflow_task (conversation_id, status);

CREATE TABLE IF NOT EXISTS group_workflow_task_participant (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES group_workflow_task(id) ON DELETE CASCADE,
    principal_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    role_key TEXT NOT NULL,
    responsibility TEXT,
    required BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (task_id, principal_id, role_key)
);

CREATE INDEX IF NOT EXISTS idx_group_workflow_task_participant_role
  ON group_workflow_task_participant (task_id, role_key);

CREATE INDEX IF NOT EXISTS idx_group_workflow_task_participant_principal
  ON group_workflow_task_participant (principal_id);

CREATE TABLE IF NOT EXISTS group_workflow_event (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversation(id) ON DELETE CASCADE,
    task_id TEXT REFERENCES group_workflow_task(id) ON DELETE SET NULL,
    source_message_id TEXT,
    actor_principal_id TEXT REFERENCES principal(id) ON DELETE SET NULL,
    kind TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(payload) = 'object'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_group_workflow_event_task_created
  ON group_workflow_event (task_id, created_at);

CREATE INDEX IF NOT EXISTS idx_group_workflow_event_conversation_created
  ON group_workflow_event (conversation_id, created_at);

COMMIT;
