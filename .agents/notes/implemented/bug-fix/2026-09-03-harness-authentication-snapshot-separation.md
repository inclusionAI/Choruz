# Agent Note: Authenticated Harness accounts survive catalog failures

Status: implemented

## Problem

A Harness can complete browser OAuth and report the signed-in account before its model catalog or exact quota windows are available. Treating the catalog as proof of authentication leaves a valid account pending and can later mislabel a parser or upstream catalog failure as a rejected login. Claude Code also exposes selectable model identifiers as `value` in current control responses while older versions use `id`.

## Decision

`choruz-harness-login` completes authentication when Claude `initialize.account` or Codex `account/read` returns a valid identity after the official OAuth completion. The login sink marks the login `verified` and the account `active` at that boundary. A separate bounded operation then discovers models and exact quota windows and publishes a complete account snapshot.

Claude model parsing accepts `value` and the legacy `id`; both become the stable `id` field in `models_json`. A catalog or quota failure is logged as a refresh failure and does not change the verified login or active account to `failed` or `error`. An incomplete refresh does not advance `probed_at`, and empty fields preserve any prior complete snapshot.

Claude quota snapshots use their stable window IDs as the semantic contract. Known IDs receive the same product label from both snapshot producers: `five_hour` is `5-hour`, and `seven_day` is `Weekly`. The Dashboard also normalizes those IDs when reading an existing snapshot, so a stored producer-specific label cannot change the account card.

The API accepts an authenticated identity with empty model and quota arrays only on the login-completion route. The runtime-host account verification route remains strict and accepts only complete, exact snapshots. Agent provisioning remains fail-closed: it can use only a model already present in an active account's `models_json`.

## Alternatives considered

**Use the model catalog as the login-success signal.** Rejected because a response-shape change or delayed catalog would invalidate credentials that the Harness has already accepted.

**Mark the account active and allow arbitrary model input until discovery succeeds.** Rejected because this would move an upstream discovery failure into Agent startup and could select a model unavailable to that account.

**Store an empty refresh as the newest exact snapshot.** Rejected because it would erase a previously verified model and quota snapshot and falsely update its freshness time.

**Trust each snapshot producer's display label.** Rejected because local and remote probes can describe the same stable window ID differently. The identifier, rather than producer wording, owns labels for known Claude windows.

## Consequences

Users can finish sign-in as soon as the Harness confirms their identity. A newly authenticated account without a successful snapshot remains visible and can be refreshed, but it cannot provision an Agent until a selectable model is verified. Parser and quota outages remain observable in logs without corrupting authentication state. Supporting both Claude model fields preserves compatibility with current and older CLI releases. Stable Claude quota IDs keep existing, local and remote account cards visually consistent.
