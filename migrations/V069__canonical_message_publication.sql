-- Canonical publication belongs to the event transaction, regardless of producer.
DROP TRIGGER IF EXISTS trg_agent_reply_sync_change ON conversation_events;
DROP FUNCTION IF EXISTS emit_agent_reply_sync_change();

CREATE FUNCTION publish_conversation_message()
RETURNS TRIGGER AS $$
DECLARE
    envelope JSONB;
    workspace TEXT;
BEGIN
    IF NEW.event_type NOT IN ('message', 'message.created', 'reply') THEN
        RETURN NEW;
    END IF;

    SELECT c.workspace_id, jsonb_build_object(
        'workspace_id', c.workspace_id,
        'conversation_id', c.id,
        'conversation_type', c.type,
        'message_id', NEW.event_id,
        'event_id', NEW.event_id,
        'sender_id', NEW.sender_id,
        'sender', jsonb_build_object('id', NEW.sender_id,
            'name', COALESCE(p.name, NEW.sender_id), 'type', COALESCE(p.type, 'agent')),
        'content', COALESCE(NEW.content, ''),
        'content_type', NEW.content_type,
        'server_seq', NEW.seq,
        'client_msg_id', NEW.client_msg_id,
        'metadata', NEW.metadata,
        'trace_id', NEW.metadata->'trace_id'
    ) INTO workspace, envelope
    FROM conversation c LEFT JOIN principal p ON p.id=NEW.sender_id
    WHERE c.id=NEW.conversation_id;

    IF NOT COALESCE(NEW.metadata->'thread' = 'true'::jsonb, false)
       OR COALESCE(NEW.metadata->'broadcast' = 'true'::jsonb, false) THEN
        UPDATE conversation SET total_msg_count=total_msg_count+1
        WHERE id=NEW.conversation_id;
        UPDATE conversation_member SET
            msg_count=(SELECT total_msg_count FROM conversation WHERE id=NEW.conversation_id),
            mention_count=0, last_viewed_at=NOW()
        WHERE conv_id=NEW.conversation_id AND principal_id=NEW.sender_id
          AND removed_at IS NULL;
    END IF;

    INSERT INTO outbox_event(id,workspace_id,principal_id,event_type,payload,created_at)
    SELECT gen_random_uuid()::text,workspace,cm.principal_id,'message.created',envelope,NEW.created_at
    FROM conversation_member cm
    WHERE cm.conv_id=NEW.conversation_id AND cm.removed_at IS NULL;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_publish_conversation_message
    AFTER INSERT ON conversation_events
    FOR EACH ROW EXECUTE FUNCTION publish_conversation_message();
