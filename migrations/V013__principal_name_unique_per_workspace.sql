-- Prevent duplicate principal names within a workspace.
--
-- Bug symptom: @mentioning `@openclaw-bridge` in a group would trigger
-- two different agents named `openclaw-bridge` at once (both showed the
-- "thinking" indicator). Root cause: the principal table had no uniqueness
-- constraint on (workspace_id, name), so the provision flow could (and
-- did) create duplicates. Mention routing in `db_service::messages` does
-- case-insensitive substring matching on name, so every duplicate fans out.
--
-- Partial-unique index pattern (same shape as V011 on company.slug):
--   scope      = (workspace_id, lower(name))
--   predicate  = deleted_at IS NULL
--
-- A disabled principal still counts — otherwise toggling `disabled=false`
-- later could re-introduce a collision.

CREATE UNIQUE INDEX IF NOT EXISTS principal_workspace_name_ci_active_key
    ON principal (workspace_id, lower(name))
    WHERE deleted_at IS NULL;
