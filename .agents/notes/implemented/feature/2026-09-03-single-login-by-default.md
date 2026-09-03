# Agent Note: One login per device by default; multiple accounts are a company switch

Status: implemented

## Problem

Most people run Choruz with one Claude Code or Codex login per computer, but the product treated every user as a multi-account user. Creating a Claude Code or Codex agent on this computer refused to proceed until an account had been added in the Harness Accounts dialog and verified, the dialog's "Add account" defaulted to signing in to a new account, and nothing scanned the login the computer already had. A single-login user therefore performed three setup steps for a choice that had exactly one answer. The requirement direction was also inverted: an agent on a remote runtime host, where an ambient login is least visible, needed no account at all. Two remote-host buttons did not work: "Refresh exact usage" always failed because probing runs on the gateway's device only, and the "use the login already on this host" option promised no sign-in but opened the sign-in panel.

## Decision

The company owns a boolean `multi_harness_accounts` (`V039__company_multi_harness_accounts.sql`, default false) that `PATCH /v1/companies/{id}` sets and every company view carries. It is the only switch; there is no plugin gate, because the dialog that shows the device's login is useful with one account too.

Off, the device's own login is the account. `ensureDefaultHarnessAccount` registers one `default` profile per company, device and harness on demand, named "Claude Code login" or "Codex login". The Harness Accounts dialog registers and, for this computer, probes it when it opens, so the user sees plan, exact usage and any login problem without adding anything. Agent provisioning (`resolveHarnessAccount` in `agent-provisioning.ts`) falls back to `defaultHarnessAccountForLaunch` for a Claude Code or Codex agent with no `harness_account_id`: a local default that has not verified is probed first; an active default with the requested model is bound; anything else leaves the binding without an account, so the agent still starts under the device's login and only the quota display is missing. Create Agent and Create Group show no account picker and no longer block on one. The `missing_harness_account` issue in the group template flow is deleted.

On, the dialog gains "Add account" (an `isolated` sign-in only, since the device's own login is already listed) and "Remove account" for every listed account, and Create Agent and Create Group show the picker. Removing the `default` row keeps it hidden while this switch remains on; selecting the device login again or turning the switch off registers it again. The picker's empty option is the device's login; it auto-selects the default account once that is active so the verified models and the account name flow into the review step, and an agent created without a choice still receives the default through the same provisioning fallback.

Removing an account hides it in Choruz (`disabled_at`) and leaves the device's credentials and profile directory in place; the dialog copy says so. "Refresh exact usage" renders only for accounts on this computer. A remote host's default account verifies through the sign-in panel; there is no probe-only connector job.

## Alternatives considered

- **A `CHORUZ_PLUGINS` entry for multiple accounts**: rejected. The plugin allowlist is an operator's build-time choice for the whole host; this is a per-company preference a user flips from the dialog, and the dialog itself stays useful when the switch is off.
- **A per-browser preference in `localStorage`**: rejected. Two people in the same company would see different Create Agent forms, and provisioning on the server could not read it.
- **Keep requiring an explicit account and only pre-select the default**: rejected. The requirement is what forced the three-step setup; with the server-side fallback the picker is a choice, not a gate, and the gate would also have to be inverted for remote hosts.
- **Probe the device's login on every Create Agent open**: rejected. A Claude probe starts the SDK and takes up to 20 seconds; Create Agent only registers the row and leaves verification to the dialog or to provisioning, which probes at most once per unverified default.
- **Delete the profile directory when an account is removed**: rejected by the product owner. Removal means "no longer shown in Choruz"; what the computer holds stays.
- **Re-register a removed device login whenever the manager opens**: rejected because Remove would only hide the row until the next modal open. Multi-account mode preserves the disabled row until the user chooses that login again.
- **A probe-only connector job so a remote host's default account verifies without a browser sign-in**: deferred. It needs a new claim/complete route pair on the host API; until then the sign-in panel is the remote verification path.

## Consequences

- A fresh install creates a Claude Code agent with no account step; the agent card shows the login's quota once the dialog or the first provisioning has probed it.
- `harness_account` gains one `default` row per company, device and harness that is used; an earlier hand-made `default` row is reused by profile kind, so nothing is duplicated.
- The dialog opens with a probe for this computer's login when it is not yet active, so the first open can take up to 20 seconds on the "verifying" state.
- `company.multi_harness_accounts` is a company column, so the flag reaches every client through `GET /v1/companies` and the bootstrap snapshot without a new route.
- A group template launched with the switch off provisions every Claude Code or Codex role under the device's login; the per-role picker returns when the switch is on.

## Testing

- `apps/web/lib/agents/agent-provisioning.test.ts`: the default account is requested with company, host, driver and model and bound; an unverified default leaves the binding without an account.
- `apps/web/lib/groups/create-group-template-flow.test.ts`: a new Claude or Codex role no longer raises `missing_harness_account`.
- `services/choruz-api-gateway/src/tests/conversations.rs::company_multi_harness_accounts_is_off_until_a_member_turns_it_on`.
- `apps/web/tests/e2e/modals.spec.ts`: a new company shows no picker and no "Add account" until the switch is on; with it on, the existing select, manage and sign-in flows.
