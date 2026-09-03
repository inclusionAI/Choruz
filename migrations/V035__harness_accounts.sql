CREATE TABLE harness_account (
    id TEXT PRIMARY KEY,
    company_id TEXT NOT NULL REFERENCES company(id) ON DELETE CASCADE,
    runtime_host_id TEXT REFERENCES runtime_host(id) ON DELETE CASCADE,
    driver_type TEXT NOT NULL CHECK (driver_type IN ('claude_terminal', 'codex_terminal')),
    name TEXT NOT NULL,
    profile_kind TEXT NOT NULL CHECK (profile_kind IN ('default', 'isolated')),
    account_fingerprint TEXT,
    subscription_type TEXT,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'active', 'reauth_required', 'error', 'disabled')),
    models_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    usage_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    last_error TEXT,
    probed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    disabled_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX harness_account_active_name_idx
    ON harness_account (company_id, COALESCE(runtime_host_id, ''), driver_type, lower(name))
    WHERE disabled_at IS NULL;

CREATE UNIQUE INDEX harness_account_default_profile_idx
    ON harness_account (company_id, COALESCE(runtime_host_id, ''), driver_type)
    WHERE profile_kind = 'default' AND disabled_at IS NULL;

CREATE UNIQUE INDEX harness_account_identity_idx
    ON harness_account (company_id, COALESCE(runtime_host_id, ''), driver_type, account_fingerprint)
    WHERE account_fingerprint IS NOT NULL AND disabled_at IS NULL;

CREATE INDEX harness_account_company_host_idx
    ON harness_account (company_id, runtime_host_id, driver_type, updated_at DESC)
    WHERE disabled_at IS NULL;

COMMENT ON TABLE harness_account IS
    'Safe metadata and exact usage snapshots for a harness login. Credentials remain in the device-local profile directory.';

CREATE OR REPLACE FUNCTION validate_runtime_binding_harness_account()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    account_id TEXT;
    account_name TEXT;
    account_profile_kind TEXT;
    selected_model TEXT;
BEGIN
    account_id := NULLIF(BTRIM(NEW.config_json->>'harness_account_id'), '');
    IF account_id IS NULL THEN
        RETURN NEW;
    END IF;
    selected_model := NULLIF(BTRIM(NEW.config_json->>'model'), '');
    SELECT name, profile_kind
      INTO account_name, account_profile_kind
      FROM harness_account
     WHERE id = account_id
       AND company_id = (SELECT workspace_id FROM principal WHERE id = NEW.agent_principal_id)
       AND driver_type = NEW.driver_type
       AND runtime_host_id IS NOT DISTINCT FROM NULLIF(BTRIM(NEW.config_json->>'runtime_host_id'), '')
       AND status = 'active'
       AND disabled_at IS NULL
       AND (selected_model IS NULL OR models_json @> jsonb_build_array(jsonb_build_object('id', selected_model)))
     FOR KEY SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'invalid active harness account or model for runtime binding'
            USING ERRCODE = '23514';
    END IF;
    NEW.config_json := jsonb_set(
        jsonb_set(NEW.config_json, '{harness_account_name}', to_jsonb(account_name), true),
        '{harness_account_profile_kind}', to_jsonb(account_profile_kind), true
    );
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_validate_runtime_binding_harness_account
BEFORE INSERT OR UPDATE OF agent_principal_id, driver_type, config_json
ON agent_runtime_bindings
FOR EACH ROW
EXECUTE FUNCTION validate_runtime_binding_harness_account();
