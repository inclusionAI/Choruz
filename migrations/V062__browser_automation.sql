CREATE TABLE browser_automation (
    workspace_id TEXT NOT NULL,
    binding_id TEXT NOT NULL REFERENCES agent_runtime_bindings(id) ON DELETE CASCADE,
    owner_id TEXT NOT NULL,
    generation BIGINT NOT NULL DEFAULT 1,
    settings JSONB,
    binding_fingerprint TEXT NOT NULL,
    PRIMARY KEY (workspace_id, binding_id)
);

ALTER TABLE browser_workflow_run ADD COLUMN revision_id TEXT;
ALTER TABLE browser_workflow_run ADD COLUMN conversation_id TEXT;
ALTER TABLE browser_workflow_run ADD COLUMN automation_generation BIGINT;
ALTER TABLE browser_workflow_run ADD COLUMN learning_generation BIGINT;
ALTER TABLE browser_workflow_run ADD COLUMN dispatch_claimed BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE browser_workflow_run ADD COLUMN notified BOOLEAN NOT NULL DEFAULT FALSE;
CREATE INDEX browser_workflow_completion_pending ON browser_workflow_run (created_at)
    WHERE automation_generation IS NOT NULL AND NOT notified;
