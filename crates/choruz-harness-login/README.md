# choruz-harness-login

Codex browser sign-in and read-only account probes for installed Claude Code and Codex CLIs. Codex uses its app-server browser login with an optional state-checked loopback callback. Claude authentication runs in the official CLI terminal, outside this crate; identity, model and exact-usage probes read the selected profile afterward.

## Entry points

- `src/lib.rs` — `LoginJob`, `LoginSink`, `run_login`, `login_binary`, `claude_signed_in`, `claude_model_catalog`, `claude_account_probe`, `codex_account_probe`

## Tests

`cargo test -p choruz-harness-login` covers Codex callback validation and probe parsing. Gateway `tests/harness_logins.rs` exercises real routes, storage and PTY transport against substituted CLI executables. These fixtures do not claim an external provider login.

## Related

- [docs/subsystems/host-and-remote.md](../../docs/subsystems/host-and-remote.md) — where each executor runs and the routes around it
- [docs/subsystems/agent-runtime.md](../../docs/subsystems/agent-runtime.md) — `harness_account` and `harness_account_login`
