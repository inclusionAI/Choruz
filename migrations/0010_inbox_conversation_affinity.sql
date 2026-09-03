-- 0010_inbox_conversation_affinity.sql
--
-- Fix cross-conversation dedup collision in agent_inbox.
--
-- Old constraint: UNIQUE (agent_id, message_id)
--   Problem: if the same message_id appears in two different conversations
--   (e.g. broadcast or relay), only the first insert succeeds. The second
--   conversation's copy is silently dropped by ON CONFLICT DO NOTHING.
--
-- New constraint: UNIQUE (agent_id, conversation_id, message_id)
--   Each (agent, conversation) pair gets its own inbox entry independently.

-- Drop the old constraint
ALTER TABLE agent_inbox
    DROP CONSTRAINT IF EXISTS agent_inbox_agent_id_message_id_key;

-- Add the new constraint with conversation_id
ALTER TABLE agent_inbox
    ADD CONSTRAINT agent_inbox_agent_conv_msg_key
    UNIQUE (agent_id, conversation_id, message_id);
