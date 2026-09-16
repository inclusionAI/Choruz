ALTER TABLE experience_policy ADD COLUMN community_settings JSONB NOT NULL
    DEFAULT '{"search":false,"automatic_trial":false,"contribute":false}'::jsonb;

ALTER TABLE experience_problem ADD COLUMN community_problem_id TEXT;

-- Private source linkage is never embedded in the exchange payload.
CREATE TABLE experience_behavior_event (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    binding_id TEXT NOT NULL REFERENCES experience_policy(binding_id) ON DELETE CASCADE,
    problem_key TEXT NOT NULL,
    source_key TEXT NOT NULL,
    episode_ref TEXT NOT NULL,
    evidence_kind TEXT NOT NULL CHECK(evidence_kind IN ('encountered','applied','effective','ineffective','recurrence')),
    solution_revision_id TEXT REFERENCES experience_revision(id) ON DELETE SET NULL,
    source_references JSONB NOT NULL,
    payload JSONB,
    public_payload JSONB,
    privacy_review JSONB,
    publication_state TEXT NOT NULL DEFAULT 'preparing'
        CHECK(publication_state IN ('preparing','local','publishing','pending','accepted','blocked','uncertain')),
    publication_url TEXT,
    publication_error TEXT,
    lease_token TEXT,
    lease_until TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(binding_id,problem_key,source_key),
    FOREIGN KEY(binding_id,problem_key) REFERENCES experience_problem(binding_id,problem_key) ON DELETE CASCADE,
    CHECK(publication_state NOT IN ('publishing','pending','accepted')
          OR (public_payload IS NOT NULL AND COALESCE(privacy_review->>'accepted'='true',false)))
);
CREATE INDEX experience_behavior_binding ON experience_behavior_event(workspace_id,binding_id,created_at);
CREATE INDEX experience_behavior_search ON experience_behavior_event USING GIN(to_tsvector('english',payload::text));
CREATE INDEX experience_behavior_outbox ON experience_behavior_event(publication_state,lease_until)
    WHERE publication_state IN ('preparing','local','publishing');

-- Only accepted public projections enter this cache, never private source linkage.
CREATE TABLE experience_community_record (
    repository TEXT NOT NULL,
    record_id TEXT NOT NULL,
    revision TEXT NOT NULL,
    blob_oid TEXT NOT NULL,
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY(repository,record_id)
);
CREATE TABLE experience_community_sync (
    repository TEXT PRIMARY KEY,
    revision TEXT,
    checked_at TIMESTAMPTZ,
    last_error TEXT
);
CREATE INDEX experience_community_search ON experience_community_record USING GIN(to_tsvector('english',payload::text));

INSERT INTO experience_behavior_event(id,workspace_id,binding_id,problem_key,source_key,episode_ref,evidence_kind,solution_revision_id,source_references)
SELECT gen_random_uuid()::text,workspace_id,binding_id,problem_key,episode_ref,episode_ref,
    CASE WHEN applied_revision_id IS NULL THEN 'encountered' ELSE 'recurrence' END,
    applied_revision_id,evidence FROM experience_problem_observation;
