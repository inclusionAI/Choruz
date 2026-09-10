ALTER TABLE experience_evaluation
    ADD COLUMN optimization JSONB,
    ADD COLUMN analyst_binding_id TEXT,
    ADD COLUMN analyst_fingerprint TEXT;
