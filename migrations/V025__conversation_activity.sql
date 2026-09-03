-- Bounded dashboard bootstrap support.
--
-- Keep one durable pointer to the latest message-like event for each
-- conversation.  The trigger runs in the writer's transaction, so every
-- message path (human, agent, imports) updates the sidebar ordering without
-- adding application-level dual writes.

CREATE TABLE IF NOT EXISTS conversation_activity (
    conversation_id  TEXT PRIMARY KEY REFERENCES conversation(id) ON DELETE CASCADE,
    last_event_seq   BIGINT,
    last_event_id    TEXT,
    last_activity_at TIMESTAMPTZ NOT NULL,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO conversation_activity (
    conversation_id,
    last_event_seq,
    last_event_id,
    last_activity_at,
    updated_at
)
SELECT
    c.id,
    latest.seq,
    latest.event_id,
    COALESCE(latest.created_at, c.updated_at),
    NOW()
FROM conversation c
LEFT JOIN LATERAL (
    SELECT ce.seq, ce.event_id, ce.created_at
    FROM conversation_events ce
    WHERE ce.conversation_id = c.id
      AND ce.event_type IN ('message', 'message.created', 'reply')
    ORDER BY ce.seq DESC
    LIMIT 1
) latest ON TRUE
ON CONFLICT (conversation_id) DO UPDATE SET
    last_event_seq = EXCLUDED.last_event_seq,
    last_event_id = EXCLUDED.last_event_id,
    last_activity_at = EXCLUDED.last_activity_at,
    updated_at = NOW();

CREATE INDEX IF NOT EXISTS idx_conversation_activity_recent
    ON conversation_activity (last_activity_at DESC, conversation_id DESC);

CREATE OR REPLACE FUNCTION update_conversation_activity()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.event_type IN ('message', 'message.created', 'reply') THEN
        INSERT INTO conversation_activity (
            conversation_id,
            last_event_seq,
            last_event_id,
            last_activity_at,
            updated_at
        ) VALUES (
            NEW.conversation_id,
            NEW.seq,
            NEW.event_id,
            NEW.created_at,
            NOW()
        )
        ON CONFLICT (conversation_id) DO UPDATE SET
            last_event_seq = EXCLUDED.last_event_seq,
            last_event_id = EXCLUDED.last_event_id,
            last_activity_at = EXCLUDED.last_activity_at,
            updated_at = NOW()
        WHERE conversation_activity.last_event_seq IS NULL
           OR EXCLUDED.last_event_seq > conversation_activity.last_event_seq;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_conversation_activity ON conversation_events;
CREATE TRIGGER trg_conversation_activity
    AFTER INSERT ON conversation_events
    FOR EACH ROW EXECUTE FUNCTION update_conversation_activity();
