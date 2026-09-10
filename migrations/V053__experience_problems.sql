CREATE TABLE experience_problem (
    binding_id TEXT NOT NULL REFERENCES experience_policy(binding_id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    problem_key TEXT NOT NULL,
    description TEXT NOT NULL,
    prompt_revision_id TEXT REFERENCES experience_revision(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (binding_id, problem_key)
);

CREATE TABLE experience_problem_observation (
    binding_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    problem_key TEXT NOT NULL,
    episode_ref TEXT NOT NULL,
    evidence JSONB NOT NULL,
    applied_revision_id TEXT REFERENCES experience_revision(id) ON DELETE SET NULL,
    report_id TEXT NOT NULL REFERENCES experience_revision(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (binding_id, problem_key, episode_ref),
    FOREIGN KEY (binding_id, problem_key) REFERENCES experience_problem(binding_id, problem_key) ON DELETE CASCADE
);
