CREATE TABLE experience_policy (
    binding_id TEXT PRIMARY KEY REFERENCES agent_runtime_bindings(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    owner_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    analyst_binding_id TEXT NOT NULL REFERENCES agent_runtime_bindings(id) ON DELETE CASCADE,
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    generation BIGINT NOT NULL DEFAULT 1,
    active_revision_id TEXT,
    source_cursor JSONB NOT NULL DEFAULT '{}'::jsonb,
    source_summary TEXT NOT NULL DEFAULT '',
    checked_at TIMESTAMPTZ,
    next_check_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    lease_until TIMESTAMPTZ,
    lease_token TEXT,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (binding_id <> analyst_binding_id)
);

CREATE INDEX experience_policy_due ON experience_policy(next_check_at)
    WHERE enabled;

CREATE TABLE experience_revision (
    id TEXT PRIMARY KEY,
    binding_id TEXT NOT NULL REFERENCES experience_policy(binding_id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    policy_generation BIGINT NOT NULL,
    parent_id TEXT REFERENCES experience_revision(id),
    source_digest TEXT NOT NULL,
    source_references JSONB NOT NULL,
    analysis TEXT NOT NULL,
    instruction TEXT NOT NULL,
    disposition TEXT NOT NULL CHECK (disposition IN ('no_change', 'candidate', 'active', 'superseded', 'rejected')),
    validation JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (binding_id, policy_generation, source_digest)
);

CREATE INDEX experience_revision_history ON experience_revision(binding_id, created_at DESC, id);

ALTER TABLE experience_policy ADD CONSTRAINT experience_policy_active_revision
    FOREIGN KEY (active_revision_id) REFERENCES experience_revision(id) ON DELETE SET NULL;
