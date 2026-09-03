-- Keep new runtime commands aligned with the bounded automatic recovery policy.
-- Existing commands retain their captured retry budget so an in-flight turn is
-- never silently shortened during deployment.
ALTER TABLE agent_commands
    ALTER COLUMN max_attempts SET DEFAULT 3;
