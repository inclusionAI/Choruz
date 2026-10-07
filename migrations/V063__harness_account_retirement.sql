ALTER TABLE harness_account
    ADD COLUMN removal_requested_at TIMESTAMPTZ,
    ADD COLUMN removal_completed_at TIMESTAMPTZ,
    ADD COLUMN removal_actor_id TEXT REFERENCES principal(id) ON DELETE SET NULL;

CREATE INDEX harness_account_pending_removal_idx
    ON harness_account (removal_requested_at, id)
    WHERE removal_requested_at IS NOT NULL AND removal_completed_at IS NULL;
