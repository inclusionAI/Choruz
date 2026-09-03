# choruz-agent-runtime

Agent runtime bindings and driver plumbing shared by the API gateway, the pipeline and the connector: `RuntimeStore` reads and writes `agent_runtime_bindings` and `conversation_runtime_policies` in PostgreSQL, `HeadlessDriver` names the coding CLIs (`Claude`, `Codex`, `Pi`, `Grok`, `OpenCode`) and parses their output, and `SessionCatalogScanner` lists native harness sessions a human can import. `services/choruz-api-gateway`, `services/choruz-pipeline` and `services/choruz-connector` depend on it.

## Entry points

- `src/binding.rs` — `DriverType`, `BindingState`, `RuntimeBinding`, `RuntimeStore`, `normalize_workspace_path`
- `src/headless.rs` — `HeadlessDriver`, `configure_command_workspace`, `harness_account_env`, `parse_output`, `validate_model`
- `src/policy.rs` — `ConversationRuntimePolicy`, `AutoMode`, `UntaggedHumanMode`, `RuntimeStore::get_policy` / `upsert_policy`
- `src/session_catalog.rs` — read-only discovery of native Claude, Codex, Pi, Grok and OpenCode sessions

## Tests

`cargo test -p choruz-agent-runtime`. `tests/runtime_store.rs` creates a temporary database per test from `CHORUZ_PG_HOST`, `CHORUZ_PG_PORT`, `CHORUZ_PG_USER` and `CHORUZ_PG_PASSWORD`, so it needs a running PostgreSQL.

## Related

- [docs/subsystems/agent-runtime.md](../../docs/subsystems/agent-runtime.md) — bindings, drivers, sessions and harness accounts
- [docs/architecture.md](../../docs/architecture.md)
