-- Imported sessions resume through PTY terminal bindings.  The terminal
-- endpoint historically defaults a missing executable to Claude, which makes
-- a Codex/Pi/Grok/OpenCode import connect and immediately exit.  Preserve
-- explicit user configuration; only backfill bindings that predate the
-- harness-specific binary field.
UPDATE agent_runtime_bindings
SET config_json = jsonb_set(
  config_json,
  '{binary_path}',
  to_jsonb(CASE driver_type
    WHEN 'claude_terminal' THEN 'claude'
    WHEN 'codex_terminal' THEN 'codex'
    WHEN 'pi_terminal' THEN 'pi'
    WHEN 'grok_terminal' THEN 'grok'
    WHEN 'opencode_terminal' THEN 'opencode'
    ELSE 'claude'
  END),
  true
)
WHERE config_json ? 'native_session_import'
  AND NOT (config_json ? 'binary_path');
