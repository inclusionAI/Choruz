-- 0016_unread_counts.sql
-- Mattermost-style server-side unread message counts.
--
-- Core formula: unread = conversation.total_msg_count - conversation_member.msg_count
-- When message sent:  conversation.total_msg_count += 1
-- When user views:    conversation_member.msg_count = conversation.total_msg_count

-- Add total message count to conversation
ALTER TABLE conversation ADD COLUMN IF NOT EXISTS total_msg_count BIGINT NOT NULL DEFAULT 0;

-- Add read tracking columns to conversation_member
ALTER TABLE conversation_member ADD COLUMN IF NOT EXISTS msg_count BIGINT NOT NULL DEFAULT 0;
ALTER TABLE conversation_member ADD COLUMN IF NOT EXISTS mention_count BIGINT NOT NULL DEFAULT 0;
ALTER TABLE conversation_member ADD COLUMN IF NOT EXISTS last_viewed_at TIMESTAMPTZ;

-- Backfill total_msg_count from existing conversation_events (if table exists)
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'conversation_events') THEN
    UPDATE conversation c SET total_msg_count = (
      SELECT COALESCE(COUNT(*), 0) FROM conversation_events ce
      WHERE ce.conversation_id = c.id
      AND ce.event_type IN ('message', 'message.created', 'reply')
    );
  END IF;
END $$;

-- Backfill msg_count to match total (mark all existing messages as read)
UPDATE conversation_member cm SET msg_count = (
  SELECT COALESCE(c.total_msg_count, 0) FROM conversation c WHERE c.id = cm.conv_id
);
