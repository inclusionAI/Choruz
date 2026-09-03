-- Per-agent inbox: messages waiting for this agent to process
CREATE TABLE IF NOT EXISTS agent_inbox (
    id BIGSERIAL PRIMARY KEY,
    agent_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    sender_name TEXT,
    content TEXT NOT NULL,
    content_type TEXT NOT NULL DEFAULT 'text',
    metadata JSONB NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'processing', 'done', 'skipped')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    claimed_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    UNIQUE (agent_id, message_id)
);

CREATE INDEX IF NOT EXISTS idx_inbox_agent_pending
    ON agent_inbox (agent_id, id)
    WHERE status = 'pending';

-- Per-agent outbox: replies waiting to be delivered
CREATE TABLE IF NOT EXISTS agent_outbox (
    id BIGSERIAL PRIMARY KEY,
    agent_id TEXT NOT NULL,
    source_conversation_id TEXT NOT NULL,
    target_conversation_id TEXT NOT NULL,
    content TEXT NOT NULL,
    content_type TEXT NOT NULL DEFAULT 'text/plain',
    metadata JSONB NOT NULL DEFAULT '{}',
    content_hash BIGINT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'delivered', 'failed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    delivered_at TIMESTAMPTZ,
    attempt_count INT NOT NULL DEFAULT 0,
    last_error TEXT,
    UNIQUE (target_conversation_id, agent_id, content_hash)
);

CREATE INDEX IF NOT EXISTS idx_outbox_pending
    ON agent_outbox (status, id)
    WHERE status = 'pending';
