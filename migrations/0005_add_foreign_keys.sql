-- 0005: Add foreign key constraints to runtime tables
-- (0003 omitted these; align with 0001 style)
-- Wrapped in DO blocks to be idempotent (safe to re-run).

DO $$ BEGIN
  ALTER TABLE agent_runtime_bindings
    ADD CONSTRAINT fk_arb_conversation
      FOREIGN KEY (conversation_id) REFERENCES conversation(id) ON DELETE CASCADE;
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE agent_runtime_bindings
    ADD CONSTRAINT fk_arb_agent
      FOREIGN KEY (agent_principal_id) REFERENCES principal(id) ON DELETE CASCADE;
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE agent_turn_leases
    ADD CONSTRAINT fk_atl_binding
      FOREIGN KEY (binding_id) REFERENCES agent_runtime_bindings(id) ON DELETE CASCADE;
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE conversation_runtime_policies
    ADD CONSTRAINT fk_crp_conversation
      FOREIGN KEY (conversation_id) REFERENCES conversation(id) ON DELETE CASCADE;
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE conversation_runtime_policies
    ADD CONSTRAINT fk_crp_reviewer_agent
      FOREIGN KEY (default_reviewer_agent_id) REFERENCES principal(id) ON DELETE SET NULL;
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;
