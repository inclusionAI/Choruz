-- Collapse agent_runtime_bindings to one row per agent.
-- Rationale: bot/agent state should be workspace-scoped (Slack/Discord bot
-- model). Agent membership in a conversation is tracked in
-- conversation_member; runtime state (workspace, session_id, driver) is now
-- globally unique per agent.
--
-- This cleanup deletes redundant rows created by earlier flows
-- (create-group auto-binding, proxy bindings). For each agent we keep the
-- row with config_json.is_primary=true if present, otherwise the row with
-- the most recent updated_at.

BEGIN;

-- Keep the "best" row per agent (is_primary first, then most recently updated).
DELETE FROM agent_runtime_bindings
WHERE id IN (
    SELECT id FROM (
        SELECT
            id,
            ROW_NUMBER() OVER (
                PARTITION BY agent_principal_id
                ORDER BY
                    ((config_json->>'is_primary')::boolean IS TRUE) DESC,
                    updated_at DESC
            ) AS rn
        FROM agent_runtime_bindings
        WHERE state <> 'disabled'
    ) ranked
    WHERE rn > 1
);

-- Add unique constraint (over non-disabled rows via partial unique index).
-- We allow multiple 'disabled' rows because they're tombstones / history.
CREATE UNIQUE INDEX IF NOT EXISTS agent_runtime_bindings_one_per_agent
    ON agent_runtime_bindings (agent_principal_id)
    WHERE state <> 'disabled';

COMMIT;
