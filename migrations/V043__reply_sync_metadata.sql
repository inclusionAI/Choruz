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
            'server_seq', NEW.seq,
            'metadata', NEW.metadata
        ),
        NEW.created_at
    FROM conversation_member cm
    WHERE cm.conv_id = NEW.conversation_id
      AND cm.removed_at IS NULL;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
