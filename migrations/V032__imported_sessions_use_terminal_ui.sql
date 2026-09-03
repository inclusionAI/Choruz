-- Imported native sessions are direct Agent conversations. They must resume
-- through the same dedicated terminal UI as a newly created Agent, rather
-- than rendering their output as ordinary chat messages.
UPDATE agent_runtime_bindings
SET
  driver_type = CASE driver_type
    WHEN 'claude_print' THEN 'claude_terminal'
    WHEN 'codex_exec' THEN 'codex_terminal'
    ELSE driver_type
  END,
  config_json = jsonb_set(
    jsonb_set(
      jsonb_set(
        jsonb_set(config_json, '{interaction_mode}', '"terminal"'::jsonb, true),
        '{external_session_mode}', '"terminal"'::jsonb, true
      ),
      '{external_session_driver_type}',
      to_jsonb(CASE driver_type
        WHEN 'claude_print' THEN 'claude_terminal'
        WHEN 'codex_exec' THEN 'codex_terminal'
        ELSE driver_type
      END),
      true
    ),
    '{native_session_import,driver_type}',
    to_jsonb(CASE driver_type
      WHEN 'claude_print' THEN 'claude_terminal'
      WHEN 'codex_exec' THEN 'codex_terminal'
      ELSE driver_type
    END),
    true
  )
WHERE config_json ? 'native_session_import';

UPDATE native_session_import
SET driver_type = CASE driver_type
  WHEN 'claude_print' THEN 'claude_terminal'
  WHEN 'codex_exec' THEN 'codex_terminal'
  ELSE driver_type
END;
