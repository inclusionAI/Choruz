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

<!-- Follow docs/testing/pr-test-policy.md#behaviour-acceptance-evidence. For each affected contract, name the user action/result and owning test/assertion, the production entry exercised and any substituted boundary. For a bugfix or guard, name the negative control. For prose-only changes, say why no product scenario applies. -->

## Ran locally

<!-- Exact commands, tested revision or dirty scope, observed results, and red/green outcomes where required. Separate deterministic fixtures, live Harness checks and manual visual evidence; static smoke checks are not live PASS. -->

## Risk

<!-- What could break and how a reader would notice; name untested or blocked acceptance paths, why they could not run, and what remains unverified. -->

<!-- If an AI agent wrote part of this PR, say so here; you have read every line and run the tests above. -->
