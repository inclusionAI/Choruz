-- ==========================================================================
-- V003: Data Migration - Legacy Messages to Conversation Events
-- ==========================================================================
-- Migrates existing messages from the old `message` table into the new
-- `conversation_events` table (latest_plan.md Section 11.2).
--
-- This migration is idempotent: it uses ON CONFLICT DO NOTHING so it can
-- be safely re-run without duplicating data.
--
-- The old `message` table is NOT dropped or altered. Both tables coexist
-- during the transition period (Phase 5 compatibility).
-- ==========================================================================

BEGIN;

-- Migrate all existing messages from the old `message` table.
-- Maps old schema fields to new conversation_events fields:
--   message.id            -> conversation_events.event_id
--   message.conv_id       -> conversation_events.conversation_id
--   message.server_seq    -> conversation_events.seq
--   message.sender_id     -> conversation_events.sender_id
--   message.content       -> conversation_events.content
--   message.content_type  -> conversation_events.content_type
--   message.metadata      -> conversation_events.metadata
--   message.created_at    -> conversation_events.created_at
--
-- event_type is determined by checking if sender is an agent (via principal table).
-- client_msg_id is populated from the old idempotency_key for dedup.

INSERT INTO conversation_events (
    conversation_id,
    seq,
    event_id,
    event_type,
    sender_id,
    content,
    content_type,
    metadata,
    client_msg_id,
    created_at
)
SELECT
    m.conv_id                                        AS conversation_id,
    m.server_seq                                     AS seq,
    m.id                                             AS event_id,
    CASE
        WHEN p.type = 'agent' THEN 'reply'
        ELSE 'message'
    END                                              AS event_type,
    m.sender_id                                      AS sender_id,
    m.content                                        AS content,
    m.content_type                                   AS content_type,
    m.metadata                                       AS metadata,
    m.idempotency_key                                AS client_msg_id,
    m.created_at                                     AS created_at
FROM message m
LEFT JOIN principal p ON p.id = m.sender_id
ORDER BY m.conv_id, m.server_seq
ON CONFLICT DO NOTHING;

-- Also migrate conversation membership data into conversation_members
-- (only runs if the target table exists; fresh installs skip this).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_tables WHERE schemaname = 'public' AND tablename = 'conversation_members') THEN
        EXECUTE '
        INSERT INTO conversation_members (
            conversation_id, principal_id, principal_type, role, joined_at, left_at
        )
        SELECT
            cm.conv_id, cm.principal_id,
            COALESCE(p.type, ''user''), cm.role, cm.joined_at, cm.removed_at
        FROM conversation_member cm
        LEFT JOIN principal p ON p.id = cm.principal_id
        ON CONFLICT DO NOTHING';
    END IF;
END $$;

-- Migrate conversations into the new conversations table (only if it exists).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_tables WHERE schemaname = 'public' AND tablename = 'conversations') THEN
        EXECUTE '
        INSERT INTO conversations (id, type, name, org_id, created_at, updated_at)
        SELECT c.id, c.type, c.name, c.workspace_id, c.created_at, c.updated_at
        FROM conversation c
        ON CONFLICT DO NOTHING';
    END IF;
END $$;

COMMIT;
