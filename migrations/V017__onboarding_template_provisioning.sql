-- V017: Product metadata for onboarding templates and durable group provisioning.
--
-- Template provenance intentionally lives outside agent_runtime_bindings.config_json.
-- Runtime bindings remain the driver/session surface; these tables describe the
-- product-level template/job lineage that must survive retries and review.

CREATE TABLE IF NOT EXISTS group_provisioning_job (
  id TEXT PRIMARY KEY,
  company_id TEXT NOT NULL REFERENCES company(id),
  requested_by TEXT NOT NULL REFERENCES principal(id),
  group_template_id TEXT NOT NULL,
  group_template_version TEXT NOT NULL,
  status TEXT NOT NULL CHECK (
    status IN (
      'validating',
      'creating_agents',
      'creating_group',
      'adding_members',
      'posting_kickoff',
      'completed',
      'completed_with_warning',
      'partial_failure',
      'failed_validation',
      'failed',
      'rolled_back',
      'canceled'
    )
  ),
  idempotency_key TEXT NOT NULL,
  plan_json JSONB NOT NULL DEFAULT '{}'::jsonb,
  step_results_json JSONB NOT NULL DEFAULT '{}'::jsonb,
  involved_agent_ids TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
  created_agent_ids TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
  created_group_id TEXT REFERENCES conversation(id),
  error_summary TEXT,
  lease_owner TEXT,
  lease_token TEXT,
  lease_expires_at TIMESTAMPTZ,
  leased_until TIMESTAMPTZ GENERATED ALWAYS AS (lease_expires_at) STORED,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  completed_at TIMESTAMPTZ,
  canceled_at TIMESTAMPTZ,
  UNIQUE (company_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS group_provisioning_job_company_status_idx
  ON group_provisioning_job (company_id, status, created_at DESC);

CREATE INDEX IF NOT EXISTS group_provisioning_job_requested_by_idx
  ON group_provisioning_job (requested_by, created_at DESC);

CREATE INDEX IF NOT EXISTS group_provisioning_job_created_group_idx
  ON group_provisioning_job (created_group_id)
  WHERE created_group_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS group_provisioning_job_active_company_idx
  ON group_provisioning_job (company_id, group_template_id, created_at DESC)
  WHERE status IN (
    'validating',
    'creating_agents',
    'creating_group',
    'adding_members',
    'posting_kickoff',
    'partial_failure',
    'failed'
  );

CREATE INDEX IF NOT EXISTS group_provisioning_job_active_involved_agents_idx
  ON group_provisioning_job USING GIN (involved_agent_ids)
  WHERE status IN (
    'validating',
    'creating_agents',
    'creating_group',
    'adding_members',
    'posting_kickoff',
    'partial_failure',
    'failed'
  );

CREATE INDEX IF NOT EXISTS group_provisioning_job_active_created_agents_idx
  ON group_provisioning_job USING GIN (created_agent_ids)
  WHERE status IN (
    'creating_agents',
    'creating_group',
    'adding_members',
    'posting_kickoff',
    'partial_failure',
    'failed'
  );

CREATE INDEX IF NOT EXISTS group_provisioning_job_lease_idx
  ON group_provisioning_job (lease_expires_at, status)
  WHERE lease_token IS NOT NULL;

CREATE TABLE IF NOT EXISTS agent_template_instance (
  agent_principal_id TEXT PRIMARY KEY REFERENCES principal(id),
  role_template_id TEXT NOT NULL,
  role_template_version TEXT NOT NULL,
  instruction_status TEXT NOT NULL CHECK (
    instruction_status IN ('template_default', 'customized', 'group_context_added')
  ),
  setup_summary JSONB NOT NULL DEFAULT '{}'::jsonb,
  workspace_mode TEXT NOT NULL CHECK (
    workspace_mode IN ('generated', 'custom', 'existing', 'none')
  ),
  selected_skills JSONB NOT NULL DEFAULT '[]'::jsonb,
  originating_job_id TEXT REFERENCES group_provisioning_job(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS agent_template_instance_template_idx
  ON agent_template_instance (role_template_id, role_template_version);

CREATE INDEX IF NOT EXISTS agent_template_instance_job_idx
  ON agent_template_instance (originating_job_id)
  WHERE originating_job_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS group_template_instance (
  group_conversation_id TEXT PRIMARY KEY REFERENCES conversation(id),
  group_template_id TEXT NOT NULL,
  group_template_version TEXT NOT NULL,
  mission TEXT NOT NULL,
  workflow JSONB NOT NULL DEFAULT '{}'::jsonb,
  kickoff_text TEXT NOT NULL,
  output_contract JSONB NOT NULL DEFAULT '{}'::jsonb,
  originating_job_id TEXT REFERENCES group_provisioning_job(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS group_template_instance_template_idx
  ON group_template_instance (group_template_id, group_template_version);

CREATE INDEX IF NOT EXISTS group_template_instance_job_idx
  ON group_template_instance (originating_job_id)
  WHERE originating_job_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS group_template_role_assignment (
  id TEXT PRIMARY KEY,
  group_conversation_id TEXT NOT NULL REFERENCES conversation(id),
  slot_id TEXT NOT NULL,
  required BOOLEAN NOT NULL DEFAULT FALSE,
  action TEXT NOT NULL CHECK (action IN ('created', 'reused', 'skipped')),
  agent_principal_id TEXT REFERENCES principal(id),
  role_template_id TEXT NOT NULL,
  role_template_version TEXT NOT NULL,
  instruction_status TEXT CHECK (
    instruction_status IN ('template_default', 'customized', 'group_context_added')
  ),
  originating_job_id TEXT REFERENCES group_provisioning_job(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CHECK (
    (action = 'skipped' AND agent_principal_id IS NULL AND instruction_status IS NULL)
    OR (action IN ('created', 'reused') AND agent_principal_id IS NOT NULL AND instruction_status IS NOT NULL)
  )
);

CREATE UNIQUE INDEX IF NOT EXISTS group_template_role_assignment_slot_idx
  ON group_template_role_assignment (group_conversation_id, slot_id);

CREATE UNIQUE INDEX IF NOT EXISTS group_template_role_assignment_agent_idx
  ON group_template_role_assignment (group_conversation_id, agent_principal_id)
  WHERE agent_principal_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS group_template_role_assignment_job_idx
  ON group_template_role_assignment (originating_job_id)
  WHERE originating_job_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS group_template_role_assignment_role_template_idx
  ON group_template_role_assignment (role_template_id, role_template_version);
