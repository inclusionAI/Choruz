-- A harness account on the gateway's own device signs in through the same
-- login handoff as a remote runtime host. NULL runtime_host_id means the API
-- gateway process runs the Harness sign-in itself; a connector never claims
-- such a row because its claim query filters on its own host id.
ALTER TABLE harness_account_login ALTER COLUMN runtime_host_id DROP NOT NULL;

COMMENT ON COLUMN harness_account_login.runtime_host_id IS
    'Runtime host whose connector runs this sign-in; NULL when the API gateway runs it on its own device.';
