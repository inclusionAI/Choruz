-- ==========================================================================
-- V002: Agent Policies table for the new message pipeline router
-- ==========================================================================
-- Per-agent, per-conversation trigger policy configuration.
-- Used by echat-router to decide whether to trigger an agent.
-- ==========================================================================

BEGIN;

CREATE TABLE IF NOT EXISTS agent_policies (
    agent_id        TEXT        NOT NULL,
    conversation_id TEXT        NOT NULL,
    auto_mode       TEXT        NOT NULL DEFAULT 'mentioned_only'
                    CHECK (auto_mode IN ('all_messages', 'mentioned_only', 'manual')),
    mention_aliases JSONB       NOT NULL DEFAULT '[]',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (agent_id, conversation_id)
);

COMMIT;
