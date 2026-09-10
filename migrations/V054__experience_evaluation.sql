CREATE TABLE experience_evaluation (
    id TEXT PRIMARY KEY,
    binding_id TEXT NOT NULL REFERENCES experience_policy(binding_id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    owner_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    revision_id TEXT NOT NULL REFERENCES experience_revision(id) ON DELETE CASCADE,
    policy_generation BIGINT NOT NULL,
    suite JSONB NOT NULL,
    candidates JSONB NOT NULL,
    context_fingerprint TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','running','completed','failed','cancelled')),
    next_case INTEGER NOT NULL DEFAULT 0 CHECK (next_case >= 0),
    results JSONB NOT NULL DEFAULT '[]',
    lease_token TEXT,
    lease_until TIMESTAMPTZ,
    error_code TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX experience_evaluation_pending ON experience_evaluation(binding_id)
    WHERE status IN ('queued','running');
CREATE INDEX experience_evaluation_history ON experience_evaluation(workspace_id,owner_id,binding_id,created_at DESC);
