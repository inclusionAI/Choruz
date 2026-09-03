CREATE TABLE IF NOT EXISTS principal (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  type TEXT NOT NULL CHECK (type IN ('human', 'agent', 'admin')),
  name TEXT NOT NULL,
  avatar_url TEXT,
  secret_hash TEXT,
  disabled BOOLEAN NOT NULL DEFAULT FALSE,
  deleted_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS conversation (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  type TEXT NOT NULL CHECK (type IN ('direct', 'group')),
  name TEXT,
  description TEXT,
  avatar_url TEXT,
  creator_id TEXT NOT NULL REFERENCES principal(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS conversation_member (
  conv_id TEXT NOT NULL REFERENCES conversation(id) ON DELETE CASCADE,
  principal_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
  joined_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  removed_at TIMESTAMPTZ,
  PRIMARY KEY (conv_id, principal_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS conversation_member_active_idx
  ON conversation_member (conv_id, principal_id)
  WHERE removed_at IS NULL;

CREATE TABLE IF NOT EXISTS message (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  conv_id TEXT NOT NULL REFERENCES conversation(id) ON DELETE CASCADE,
  sender_id TEXT NOT NULL REFERENCES principal(id),
  content TEXT NOT NULL,
  content_type TEXT NOT NULL,
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
  server_seq BIGINT NOT NULL,
  idempotency_key TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS message_idempotency_idx
  ON message (workspace_id, conv_id, sender_id, idempotency_key);

CREATE UNIQUE INDEX IF NOT EXISTS message_server_seq_idx
  ON message (conv_id, server_seq);

CREATE TABLE IF NOT EXISTS receipt (
  principal_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
  conv_id TEXT NOT NULL REFERENCES conversation(id) ON DELETE CASCADE,
  last_read_msg_id TEXT,
  last_read_seq BIGINT NOT NULL DEFAULT 0,
  last_read_at TIMESTAMPTZ,
  PRIMARY KEY (principal_id, conv_id)
);

CREATE TABLE IF NOT EXISTS presence (
  principal_id TEXT PRIMARY KEY REFERENCES principal(id) ON DELETE CASCADE,
  status TEXT NOT NULL CHECK (status IN ('online', 'offline', 'busy')),
  last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS audit_log (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  actor_id TEXT NOT NULL REFERENCES principal(id),
  action TEXT NOT NULL,
  target_type TEXT NOT NULL,
  target_id TEXT NOT NULL,
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS outbox_event (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  principal_id TEXT NOT NULL REFERENCES principal(id),
  event_type TEXT NOT NULL,
  payload JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  acknowledged_at TIMESTAMPTZ
);
