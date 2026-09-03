-- Agent replies are committed directly to conversation_events by the pipeline
-- writer. Unlike human messages, they do not pass through outbox_event, so the
-- V026 trigger could not project them into the per-principal dashboard feed.
-- Project replies in the same transaction as their canonical event insert.

CREATE OR REPLACE FUNCTION emit_agent_reply_sync_change()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.event_type <> 'reply' THEN
        RETURN NEW;
    END IF;

    INSERT INTO sync_change (
        principal_id, event_type, entity_type, entity_id,
        conversation_id, payload, created_at
    )
    SELECT
        cm.principal_id,
        'message.created',
        'message',
        NEW.event_id,
        NEW.conversation_id,
        jsonb_build_object(
            'message_id', NEW.event_id,
            'event_id', NEW.event_id,
            'conversation_id', NEW.conversation_id,
            'sender_id', NEW.sender_id,
            'content', NEW.content,
            'content_type', NEW.content_type,
            'server_seq', NEW.seq
        ),
        NEW.created_at
    FROM conversation_member cm
    WHERE cm.conv_id = NEW.conversation_id
      AND cm.removed_at IS NULL;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_agent_reply_sync_change ON conversation_events;
CREATE TRIGGER trg_agent_reply_sync_change
    AFTER INSERT ON conversation_events
    FOR EACH ROW EXECUTE FUNCTION emit_agent_reply_sync_change();
