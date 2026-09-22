ALTER TABLE experience_policy ADD COLUMN decision_settings JSONB;
ALTER TABLE experience_policy ADD COLUMN active_decision_revision_id TEXT REFERENCES experience_revision(id) ON DELETE SET NULL;

CREATE TABLE experience_decision_trial (
    workspace_id TEXT NOT NULL,
    binding_id TEXT NOT NULL REFERENCES experience_policy(binding_id) ON DELETE CASCADE,
    corpus TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, binding_id, corpus)
);
