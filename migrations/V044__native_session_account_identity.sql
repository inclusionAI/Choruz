ALTER TABLE native_session_import
    ADD COLUMN harness_account_id TEXT;

ALTER TABLE native_session_import
    DROP CONSTRAINT native_session_import_device_session_key;

ALTER TABLE native_session_import
    ADD CONSTRAINT native_session_import_profile_session_key
    UNIQUE NULLS NOT DISTINCT
        (runtime_host_id, harness_account_id, workspace_path, driver_type, native_session_id);

COMMENT ON COLUMN native_session_import.harness_account_id IS
    'Isolated profile identity at import; NULL denotes the device default store. Retained after account removal to preserve import identity.';
