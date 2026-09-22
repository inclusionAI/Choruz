CREATE TABLE browser_workflow_grant (
    workspace_id TEXT NOT NULL,
    binding_id TEXT NOT NULL REFERENCES agent_runtime_bindings(id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    revision_id TEXT NOT NULL REFERENCES experience_revision(id) ON DELETE CASCADE,
    source_run_id TEXT NOT NULL,
    binding_fingerprint TEXT NOT NULL,
    manifest JSONB NOT NULL,
    applicability TEXT NOT NULL,
    remaining_runs INTEGER NOT NULL CHECK (remaining_runs BETWEEN 0 AND 50),
    revoked BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (workspace_id, binding_id, id),
    FOREIGN KEY (workspace_id, binding_id, source_run_id)
        REFERENCES browser_workflow_run(workspace_id, binding_id, id)
);

ALTER TABLE browser_workflow_run ADD COLUMN grant_id TEXT;
ALTER TABLE browser_workflow_run ADD COLUMN binding_fingerprint TEXT;
