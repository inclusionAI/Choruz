# Agent Note: Manage harness accounts by device

Status: implemented

## Problem

Create Agent combined account setup, browser authorization, repair, exact usage refresh, and agent configuration in one form. Repeating these device-level operations for each Agent made multi-account use hard to understand and increased accidental account changes while provisioning.

## Decision

Harness account lifecycle operations live in the Actions menu under Manage Harness Accounts. The dialog scopes its account list to one device and one supported Harness, where it adds accounts, runs local or remote browser login, verifies identity and models, refreshes exact usage, and removes accounts. Removing an account atomically stops its dependent runtime bindings before hiding it. Create Agent retains device, active account, and verified model selection only. Existing account rows, isolated profiles, remote browser authorization, and runtime binding fields remain unchanged.

## Alternatives considered

**Keep account setup in Create Agent.** Rejected because account login and quota inspection are device operations that outlive any one Agent.

**Create a new account or binding data model.** Rejected because existing account isolation, authorization, and binding ownership already provide the required boundary.

**Show every device and Harness account in one unfiltered list.** Rejected because credentials and exact usage belong to a specific device and Harness context.

## Consequences

An account must be active before it is selectable while creating an Agent. Users visit the management dialog when they need a new login, repair, or quota refresh. Existing Agents retain their bound accounts and remote authorization flows continue to keep credentials on their owning device.
