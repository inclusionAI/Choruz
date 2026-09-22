CREATE TABLE browser_workflow_run (
    workspace_id TEXT NOT NULL,
    binding_id TEXT NOT NULL REFERENCES agent_runtime_bindings(id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('running', 'finished', 'cancelled')),
    result JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '150 seconds',
    PRIMARY KEY (workspace_id, binding_id, id)
);
