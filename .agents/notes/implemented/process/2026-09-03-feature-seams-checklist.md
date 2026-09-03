# Agent Note: Feature seams checklist

Status: implemented

## Problem

A contributor adding a feature has to find, on their own, the one extension point for each horizontal concern: where routes are registered, which `require_*` helper gates a handler, how a metric reaches `/metrics`, which selector rule makes CI run a new Playwright spec, whether a rollout switch exists at all. Those facts are spread over `AGENTS.md`, the subsystem pages, the test policy, `docs/plugins.md` and the code itself, and nothing lists them in the order a feature meets them. The observed failure is a feature that re-implements a seam instead of plugging into it: a handler with private SQL or a private auth check, a spec no rule selects, an environment variable invented as a feature flag. A reviewer catches each of these only by remembering to look, and the pull request template gives them nowhere to see which seams the author claims to have covered.

## Decision

[docs/adding-a-feature.md](../../../../docs/adding-a-feature.md) is the single owner of the seam list: one section per layer (Domain, Interface, Persistence, Authorization, Observability, Testing, Rollout, Compatibility, Documentation), each naming the extension point by path and symbol, one shipped example (the channel Tasks board wherever it applies) and what breaks or what CI catches when the seam is skipped. Every fact in it is read from the tree, and the page links the owning document instead of restating a rule.

The page is wired into the pull request flow at four points:

- `AGENTS.md` links it from the Documentation section and from the "Pick the change type first" convention: a `feature`, `api`, `database`, `security` or `auth` change walks it and records the seams in the pull request.
- `.github/PULL_REQUEST_TEMPLATE.md` carries a `## Seams touched` checklist after `## Agent Note`; `refactor`, `docs` and `chore` pull requests may delete it, and an unticked seam carries a one-line reason.
- [choruz-pr](../../../skills/choruz-pr/SKILL.md) step 1 tells the author to walk the seams before writing tests and adds the section to its checklist; the skill links the page rather than repeating the seams.
- [choruz-code-review](../../../skills/choruz-code-review/SKILL.md) gains one manual check: the "Seams touched" section matches the diff, and a new route without a `require_*` helper, a new table without `workspace_id` or a new spec without a selector rule is a finding.

`docs/architecture.md` and `docs/AGENTS.md` point to the page so it is reachable from the map and the tier index.

## Alternatives considered

- **Put the seam list inside the `choruz-pr` skill**: rejected because a skill is an operating procedure that links its sources of truth ([AGENTS.md](../../../../AGENTS.md#skills)); the seams are current-state reference that reviewers, subsystem pages and `docs/AGENTS.md` also need to reach, and a list living in a skill would drift from the code the way the archived RFCs did. The skill links the page and owns only the procedure around it.
- **Enforce the seams with a CI check** (a script that fails when a new `.route(` line has no `require_*` call in its handler, a new `CREATE TABLE` has no `workspace_id`, or a new spec has no selector rule): rejected for this change because each rule has legitimate exceptions today (`handlers_channel_tasks.rs` authenticates with `authenticated_principal` and gates membership in `DbService`; `group_workflow_task` scopes through `conversation_id`; the P0 specs are selected by name), so a mechanical gate would either block correct code or need an allowlist that decays into noise. The template plus the review check keep the judgement with a person while the page keeps the facts in one place; a narrow gate can be added later for any rule that turns out to have no exceptions.
- **Fold the seams into `docs/architecture.md`**: rejected because the architecture page is the ordered map of how the system works, read before any change, and the seam list is a procedure read only when adding a feature; merging them would make the map longer for every reader to serve one.
- **A section per seam in each subsystem page**: rejected because a feature crosses subsystems, and a contributor would have to read eleven pages to assemble the list this page gives in one.

## Consequences

- A feature-type pull request now carries one more section, and its reviewer has a concrete list to check the diff against. That is deliberate friction, in the same spirit as the Agent Note requirement.
- The page names symbols (`router_with_runtime`, `request_logging_middleware`, `append_channel_task_metrics`, `RULES`, `plugin_enabled`); a rename or a move updates it in the same change under the docs rule in `AGENTS.md`. The metrics section describes the hand-written `/metrics` body and states that the counters are moving to a shared registry in `crates/choruz-common`; when that lands, the Observability section is rewritten to the registry.
- There is still no mechanical enforcement: a pull request that ticks a seam it did not touch passes CI. The review check is the only guard, and the [choruz-code-review](../../../skills/choruz-code-review/SKILL.md) skill names it.
- The Rollout section states plainly that `CHORUZ_PLUGINS` is the only switch and that no per-user feature flag or admin surface exists, so a contributor stops looking for one and applies the pre-release stance instead.

## Related

- [Agent Notes, AGENTS.md and repository skills](2026-09-03-agent-notes-and-skills.md) owns the tiering that places this page in `docs/` and the checklist wiring in `AGENTS.md`.
- [Workspace-scoped isolation](../architecture/2026-08-18-workspace-scoped-isolation.md) owns the `workspace_id` rule the Persistence and Authorization sections point to.
- [docs/testing/pr-test-policy.md](../../../../docs/testing/pr-test-policy.md) owns the change types the checklist is keyed on.
