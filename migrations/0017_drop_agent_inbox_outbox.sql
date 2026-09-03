-- Drop agent_inbox and agent_outbox tables (runner/consumer legacy).
-- These tables were the message-delivery queue for the old runner + consumer
-- stack. Since commit 5f4a412 (remove deprecated echat-runner + echat-consumer
-- stack) the runner and consumer crates are gone, and no code path reads or
-- writes these tables. DROP is safe.

DROP TABLE IF EXISTS agent_outbox CASCADE;
DROP TABLE IF EXISTS agent_inbox CASCADE;
