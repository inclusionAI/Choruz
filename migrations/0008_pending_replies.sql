-- Per-message delivery tracking for session reader → group chat delivery.
-- Replaces pure in-memory offset tracking with durable per-reply state.
CREATE TABLE IF NOT EXISTS pending_replies (
    id TEXT PRIMARY KEY,
    binding_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    content TEXT NOT NULL,
    content_hash BIGINT NOT NULL,
    source_file TEXT,
    source_offset BIGINT,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'delivered', 'failed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    delivered_at TIMESTAMPTZ,
    last_error TEXT
);

-- Partial index for fast recovery queries on startup
CREATE INDEX IF NOT EXISTS idx_pending_replies_status
    ON pending_replies(status) WHERE status = 'pending';

-- Prevent duplicate extraction of the same content for the same conversation
CREATE UNIQUE INDEX IF NOT EXISTS idx_pending_replies_dedup
    ON pending_replies(conversation_id, agent_id, content_hash);
