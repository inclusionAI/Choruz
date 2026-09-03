# choruz-harness-login

The browser sign-in driver for Claude Code and Codex accounts. `run_login` starts the Harness binary (`CHORUZ_CLAUDE_BINARY` / `CHORUZ_CODEX_BINARY`, else `claude` / `codex`), walks its login protocol and reports through a `LoginSink`: the authorization link, any callback the user pastes back, and the verified `AccountProbe` (identity fingerprint, plan, models, exact quota windows). Claude accepts the complete `authorization-code#state` value shown by its manual flow. Codex uses the app-server's standard browser login. A local browser completes its loopback callback automatically; for a remote host the user copies the browser's complete localhost callback URL and the connector forwards its validated code and state to the loopback listener it owns. The API gateway runs the driver in-process for accounts on its own device; `choruz-connector` runs it on a remote runtime host. Credentials never pass through the sink: the Harness writes them into the account's profile directory.

## Entry points

- `src/lib.rs` — `LoginJob`, `LoginSink`, `run_login`, `login_binary`, `callback_code_and_state`, `claude_account_probe`, `codex_account_probe`

## Tests

`cargo test -p choruz-harness-login`; the tests cover callback parsing, loopback-only Codex callback forwarding, and probe shaping, no Harness process. The gateway's `tests/harness_logins.rs` drives both login protocols against protocol-faithful fake executables, including delayed Claude account readiness.

## Related

- [docs/subsystems/host-and-remote.md](../../docs/subsystems/host-and-remote.md) — where each executor runs and the routes around it
- [docs/subsystems/agent-runtime.md](../../docs/subsystems/agent-runtime.md) — `harness_account` and `harness_account_login`
