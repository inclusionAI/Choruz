ALTER TABLE agent_runtime_bindings
  DROP CONSTRAINT IF EXISTS agent_runtime_bindings_driver_type_check;

ALTER TABLE agent_runtime_bindings
  ADD CONSTRAINT agent_runtime_bindings_driver_type_check
  CHECK (
    driver_type IN (
      'claude_print',
      'claude_terminal',
      'codex_exec',
      'codex_app_server',
      'acp'
    )
  );
