CREATE TABLE IF NOT EXISTS agent_runtime_bindings (
  id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL,
  agent_principal_id TEXT NOT NULL,
  driver_type TEXT NOT NULL CHECK (
    driver_type IN ('claude_print', 'codex_exec', 'codex_app_server', 'acp')
  ),
  workspace_path TEXT NOT NULL,
  git_worktree_path TEXT,
  external_session_id TEXT,
  external_thread_id TEXT,
  last_event_cursor BIGINT NOT NULL DEFAULT 0,
  last_acked_event_cursor BIGINT NOT NULL DEFAULT 0,
  last_seen_server_seq BIGINT NOT NULL DEFAULT 0,
  state TEXT NOT NULL DEFAULT 'idle' CHECK (
    state IN ('idle', 'running', 'paused', 'disabled', 'error')
  ),
  last_error TEXT,
  in_flight_turn_id TEXT,
  last_trigger_message_id TEXT,
  config_json JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE (conversation_id, agent_principal_id)
);

CREATE INDEX IF NOT EXISTS agent_runtime_bindings_conversation_idx
  ON agent_runtime_bindings (conversation_id);

CREATE INDEX IF NOT EXISTS agent_runtime_bindings_agent_idx
  ON agent_runtime_bindings (agent_principal_id);

CREATE INDEX IF NOT EXISTS agent_runtime_bindings_state_idx
  ON agent_runtime_bindings (state);

CREATE TABLE IF NOT EXISTS agent_turn_leases (
  id TEXT PRIMARY KEY,
  binding_id TEXT NOT NULL UNIQUE,
  lease_owner TEXT NOT NULL,
  lease_token TEXT NOT NULL,
  lease_until TIMESTAMPTZ NOT NULL,
  turn_key TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS agent_turn_leases_owner_idx
  ON agent_turn_leases (lease_owner);

CREATE TABLE IF NOT EXISTS conversation_runtime_policies (
  conversation_id TEXT PRIMARY KEY,
  auto_mode TEXT NOT NULL DEFAULT 'mentioned_only' CHECK (
    auto_mode IN ('disabled', 'mentioned_only', 'metadata_only')
  ),
  max_auto_turns INT NOT NULL DEFAULT 4,
  require_human_after_n_turns INT NOT NULL DEFAULT 4,
  allow_agent_to_agent BOOLEAN NOT NULL DEFAULT FALSE,
  allow_file_write BOOLEAN NOT NULL DEFAULT TRUE,
  default_reviewer_agent_id TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS conversation_runtime_policies_mode_idx
  ON conversation_runtime_policies (auto_mode);
