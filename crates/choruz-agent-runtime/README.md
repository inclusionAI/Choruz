# Choruz agent runtime

Prepare CLI arguments and account environments, interpret headless output, and discover native sessions without starting the Choruz platform or connecting to PostgreSQL. This library does not bundle agent binaries, perform sign-in, persist platform bindings or start a supervisor.

`HeadlessDriver` owns driver-specific arguments and output parsing. `configure_command_workspace` and `prepare_harness_account_env` preserve the selected workspace and account profile. The caller owns permission policy and process lifetime: argument helpers encode Choruz's unattended execution settings, not a sandbox suitable for arbitrary untrusted work.

`SessionCatalogScanner` reads the selected native account stores. `latest_native_session` is narrower: it finds a workspace-scoped session only where the driver can establish that association; it never guesses a Codex binding from global session files. Read errors and missing sessions retain the distinctions in the returned types.

## Validate standalone use

The crate's tests need no platform database:

```sh
cargo test -p choruz-agent-runtime
```

Use a path dependency on this directory to consume it from another Cargo project. Its only workspace dependency is `choruz-common`; neither crate depends on a PostgreSQL client. Both can be packaged together without publishing them:

```sh
cargo package -p choruz-common -p choruz-agent-runtime --allow-dirty
```

The platform's `RuntimeStore` and conversation policy persistence live in `choruz-application::runtime_store`. Native discovery does not authorize a binding update; the platform adapter checks the captured binding identity before persisting a result.
