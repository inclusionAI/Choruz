CREATE TABLE experience_stage_checkpoint (
    workspace_id TEXT NOT NULL,
    binding_id TEXT NOT NULL REFERENCES experience_policy(binding_id) ON DELETE CASCADE,
    policy_generation BIGINT NOT NULL,
    stage TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    claim_token TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('running', 'completed', 'retryable', 'uncertain')),
    result JSONB,
    attempts INTEGER NOT NULL DEFAULT 1 CHECK (attempts BETWEEN 1 AND 3),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, binding_id, policy_generation, stage, request_digest),
    CHECK ((state = 'completed') = (result IS NOT NULL))
);

ALTER TABLE experience_decision_trial ADD COLUMN policy_generation BIGINT NOT NULL DEFAULT 0;

CREATE TABLE experience_window_checkpoint (
    workspace_id TEXT NOT NULL,
    binding_id TEXT NOT NULL REFERENCES experience_policy(binding_id) ON DELETE CASCADE,
    policy_generation BIGINT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, binding_id, policy_generation)
);

CREATE FUNCTION clear_cancelled_learning_checkpoints() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.generation <> OLD.generation OR NOT NEW.enabled THEN
        DELETE FROM experience_stage_checkpoint WHERE binding_id=NEW.binding_id AND workspace_id=NEW.workspace_id;
        DELETE FROM experience_window_checkpoint WHERE binding_id=NEW.binding_id AND workspace_id=NEW.workspace_id;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_clear_cancelled_learning_checkpoints AFTER UPDATE ON experience_policy
    FOR EACH ROW EXECUTE FUNCTION clear_cancelled_learning_checkpoints();
