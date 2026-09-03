-- Create company and company_member tables.
-- These tables were used by application code but never had a CREATE TABLE migration.
-- Uses IF NOT EXISTS for safety on environments where tables were created manually.

CREATE TABLE IF NOT EXISTS company (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  slug TEXT NOT NULL UNIQUE,
  description TEXT,
  avatar_url TEXT,
  owner_id TEXT NOT NULL REFERENCES principal(id),
  agents_active BOOLEAN NOT NULL DEFAULT FALSE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS company_member (
  company_id TEXT NOT NULL REFERENCES company(id) ON DELETE CASCADE,
  principal_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
  joined_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (company_id, principal_id)
);
