-- Notify triggers for event-driven pipeline scheduling.
-- Replaces polling with instant wake-up on new outbox entries and commands.

-- Notify when a new outbox entry is inserted
CREATE OR REPLACE FUNCTION notify_outbox_insert() RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify('echat_outbox', NEW.id::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_outbox_notify ON event_outbox;
CREATE TRIGGER trg_outbox_notify
    AFTER INSERT ON event_outbox
    FOR EACH ROW EXECUTE FUNCTION notify_outbox_insert();

-- Notify when a new agent command is inserted
CREATE OR REPLACE FUNCTION notify_command_insert() RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify('echat_commands', NEW.command_id::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_command_notify ON agent_commands;
CREATE TRIGGER trg_command_notify
    AFTER INSERT ON agent_commands
    FOR EACH ROW EXECUTE FUNCTION notify_command_insert();
