CREATE TABLE IF NOT EXISTS agent_cron_job (
    id TEXT PRIMARY KEY DEFAULT gen_random_uuid()::text,
    agent_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    schedule_type TEXT NOT NULL CHECK (schedule_type IN ('at', 'every', 'cron')),
    schedule_value TEXT NOT NULL,
    schedule_timezone TEXT,
    message TEXT NOT NULL,
    session_target TEXT NOT NULL DEFAULT 'main' CHECK (session_target IN ('main', 'isolated')),
    delivery_mode TEXT NOT NULL DEFAULT 'announce' CHECK (delivery_mode IN ('announce', 'none')),
    enabled BOOLEAN NOT NULL DEFAULT true,
    running_at TIMESTAMPTZ,
    last_run_at TIMESTAMPTZ,
    next_run_at TIMESTAMPTZ,
    last_status TEXT,
    last_error TEXT,
    last_duration_ms BIGINT,
    consecutive_errors INT NOT NULL DEFAULT 0,
    timeout_seconds INT NOT NULL DEFAULT 600,
    delete_after_run BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_cron_agent ON agent_cron_job (agent_id);
CREATE INDEX IF NOT EXISTS idx_cron_next_run ON agent_cron_job (next_run_at) WHERE enabled = true;
