-- Idempotent mapping between a native harness session and its Choruz Agent/DM.

CREATE TABLE IF NOT EXISTS native_session_import (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL REFERENCES company(id) ON DELETE CASCADE,
  workspace_path TEXT NOT NULL,
  driver_type TEXT NOT NULL,
  native_session_id TEXT NOT NULL,
  agent_principal_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
  conversation_id TEXT NOT NULL REFERENCES conversation(id) ON DELETE CASCADE,
  binding_id TEXT NOT NULL REFERENCES agent_runtime_bindings(id) ON DELETE CASCADE,
  imported_by TEXT NOT NULL REFERENCES principal(id),
  native_title TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE (workspace_path, driver_type, native_session_id),
  UNIQUE (agent_principal_id),
  UNIQUE (conversation_id),
  UNIQUE (binding_id)
);

CREATE INDEX IF NOT EXISTS native_session_import_workspace_idx
  ON native_session_import (company_id, workspace_path, updated_at DESC);
