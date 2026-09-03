-- V014: globally-unique username for human signups
--
-- The pre-existing `principal_workspace_name_ci_active_key` makes
-- (workspace_id, lower(name)) unique while deleted_at IS NULL.  That's
-- right for agents (each company has its own agent named "frontend-dev"
-- etc.) but wrong for human users: when a person types their username
-- at the login form we have no workspace_id yet, so we must be able to
-- find them by name alone.  This partial unique index enforces that
-- humans can't share a username globally.
CREATE UNIQUE INDEX IF NOT EXISTS principal_human_username_unique_idx
  ON principal (lower(name))
  WHERE type = 'human' AND deleted_at IS NULL;
