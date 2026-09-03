-- 0006: Add max_workflow_turns to conversation_runtime_policies
-- Used as the turn cap when allow_agent_to_agent is enabled (workflow mode).
-- IF NOT EXISTS makes this idempotent (safe to re-run).

ALTER TABLE conversation_runtime_policies
  ADD COLUMN IF NOT EXISTS max_workflow_turns INT NOT NULL DEFAULT 20;
