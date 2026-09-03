-- V039: A company opts in to more than one harness account per device.
--
-- Off (the default), every Claude Code or Codex agent runs under the login
-- its device already has; Choruz registers and verifies that login as the
-- device's `default` harness account by itself. On, the Harness Accounts
-- dialog also adds isolated sign-ins and Create Agent chooses among them.

ALTER TABLE company
    ADD COLUMN multi_harness_accounts BOOLEAN NOT NULL DEFAULT FALSE;

COMMENT ON COLUMN company.multi_harness_accounts IS
    'When false, agents use the login their device already has and the UI offers no account choice.';
