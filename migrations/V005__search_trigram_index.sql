-- Enable pg_trgm extension for ILIKE index support
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- GIN trigram index on conversation_events.content
-- PostgreSQL automatically uses this for ILIKE '%pattern%' queries
-- Note: not using CONCURRENTLY so this can run inside a transaction (test harness).
-- For large production tables, run manually with CONCURRENTLY if needed.
CREATE INDEX IF NOT EXISTS idx_ce_content_trgm
    ON conversation_events USING gin(content gin_trgm_ops);
