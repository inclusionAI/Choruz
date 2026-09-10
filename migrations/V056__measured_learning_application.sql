ALTER TABLE experience_policy ADD COLUMN optimization_settings JSONB;
ALTER TABLE experience_policy ADD COLUMN optimization_error TEXT;
ALTER TABLE experience_evaluation
    ADD COLUMN automatic BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN auto_apply BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN final_review_reserved BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN application_status TEXT NOT NULL DEFAULT 'not_requested'
        CHECK (application_status IN ('not_requested','pending','no_improvement','review_rejected','applied','failed','cancelled')),
    ADD COLUMN applied_revision_id TEXT REFERENCES experience_revision(id) ON DELETE SET NULL;
CREATE UNIQUE INDEX experience_evaluation_automatic_once
    ON experience_evaluation(binding_id,policy_generation,revision_id) WHERE automatic;
