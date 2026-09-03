-- Phase 3B clean-breaking database cutover.
--
-- Historical migrations remain immutable. This forward migration changes only
-- active schema/runtime identifiers and system-owned persisted discriminators.

BEGIN;

-- A prior attempt can have committed this file before the migration runner
-- records its filename. Handle that marker-gap safely, but fail closed when a
-- manual/interrupted partial rename leaves conflicting bridge columns behind.
DO $$
DECLARE
    has_old_column BOOLEAN;
    has_new_column BOOLEAN;
BEGIN
    SELECT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'bridge_channel_mappings'
          AND column_name = 'echat_conversation_id'
    ) INTO has_old_column;
    SELECT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'bridge_channel_mappings'
          AND column_name = 'choruz_conversation_id'
    ) INTO has_new_column;

    IF has_old_column AND has_new_column THEN
        RAISE EXCEPTION
            'ambiguous bridge mapping cutover: both old and new conversation columns exist';
    ELSIF has_old_column THEN
        ALTER TABLE bridge_channel_mappings
            RENAME COLUMN echat_conversation_id TO choruz_conversation_id;
    ELSIF NOT has_new_column THEN
        RAISE EXCEPTION
            'bridge_channel_mappings has neither echat_conversation_id nor choruz_conversation_id';
    END IF;
END;
$$;

-- The original index automatically follows its column rename but retains its
-- old name. Rename it once; fail rather than guessing when both names exist.
DO $$
DECLARE
    has_old_index BOOLEAN;
    has_new_index BOOLEAN;
BEGIN
    SELECT EXISTS (
        SELECT 1 FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND c.relkind = 'i'
          AND c.relname = 'idx_bridge_mappings_echat_conv'
    ) INTO has_old_index;
    SELECT EXISTS (
        SELECT 1 FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND c.relkind = 'i'
          AND c.relname = 'idx_bridge_mappings_choruz_conv'
    ) INTO has_new_index;

    IF has_old_index AND has_new_index THEN
        RAISE EXCEPTION
            'ambiguous bridge mapping cutover: both old and new conversation indexes exist';
    ELSIF has_old_index THEN
        ALTER INDEX idx_bridge_mappings_echat_conv
            RENAME TO idx_bridge_mappings_choruz_conv;
    ELSIF NOT has_new_index THEN
        CREATE INDEX idx_bridge_mappings_choruz_conv
            ON bridge_channel_mappings (choruz_conversation_id);
    END IF;
END;
$$;

-- Replace trigger implementations atomically. Existing trigger identities are
-- preserved; only their active notification channels change.
CREATE OR REPLACE FUNCTION notify_outbox_insert() RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify('choruz_outbox', NEW.id::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION notify_command_insert() RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify('choruz_commands', NEW.command_id::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION notify_file_outbox() RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify('choruz_file_outbox', NEW.id);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Only system-owned discriminator fields change. Do not rewrite arbitrary
-- user-authored content or free-form JSON metadata.
UPDATE conversation_events
SET content_type = 'application/vnd.choruz.channel-task+json'
WHERE content_type = 'application/vnd.echat.channel-task+json';

UPDATE message
SET content_type = 'application/vnd.choruz.channel-task+json'
WHERE content_type = 'application/vnd.echat.channel-task+json';

UPDATE event_outbox
SET payload = jsonb_set(
    payload,
    '{content_type}',
    '"application/vnd.choruz.channel-task+json"'::jsonb,
    false
)
WHERE payload ->> 'content_type' = 'application/vnd.echat.channel-task+json';

UPDATE outbox_event
SET payload = jsonb_set(
    payload,
    '{content_type}',
    '"application/vnd.choruz.channel-task+json"'::jsonb,
    false
)
WHERE payload ->> 'content_type' = 'application/vnd.echat.channel-task+json';

COMMIT;
