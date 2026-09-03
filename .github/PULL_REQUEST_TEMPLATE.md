<!-- See docs/testing/pr-test-policy.md for what each type has to add and what CI runs. -->

## Type

<!-- Pick one. CI decides what to run from the changed files; the type tells reviewers what tests to expect. -->

- [ ] `feature` — new behaviour
- [ ] `bugfix` — fixes a bug
- [ ] `refactor` — no behaviour change
- [ ] `ui` — interface change
- [ ] `api` / `database` — API contract, schema or migration (add the `database` label to run the full e2e suite)
- [ ] `security` / `auth` — permissions, sessions, secrets (add the `security` label to run the full e2e suite)
- [ ] `ci` / `build` / `deps`
- [ ] `docs` / `chore`

## Agent Note

<!-- Link the note this change adds or updates under .agents/notes/, or write "none: mechanical/local edit" and say why. Rules: .agents/notes/README.md#when-to-write-one -->

## Seams touched

<!-- feature / api / database / security / auth: walk docs/adding-a-feature.md and tick the seams this diff plugs into; an unticked seam gets a one-line reason. refactor / docs / chore PRs may delete this section. -->

- [ ] Interface: routes and their `openapi/` entries
- [ ] Persistence: migration `V0NN__name.sql`, `workspace_id`, `docs/data-model.md`
- [ ] Authorization: which `require_*` helper; `record_audit` if a mutation
- [ ] Observability: metric or log added
- [ ] Testing: which tests; which `select_e2e_specs.py` rule
- [ ] Rollout: `CHORUZ_PLUGINS` gate, or none
- [ ] Compatibility: contract, sync-feed or bootstrap version kept
- [ ] Documentation: subsystem page

## Summary

<!-- What changes and why. Link the issue if there is one. -->

## Tests

<!-- Tests added or updated. For a bugfix, name the regression test. If none, say why. -->

## Ran locally

<!-- e.g. cargo test -p choruz-api-gateway, pnpm web:test, pnpm web:e2e -- tests/e2e/git-graph.spec.ts -->

## Risk

<!-- What could break, and how a reader would notice. -->

<!-- If an AI agent wrote part of this PR, say so here; you have read every line and run the tests above. -->
