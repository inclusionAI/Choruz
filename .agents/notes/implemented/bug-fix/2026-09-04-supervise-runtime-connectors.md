# Agent Note: Supervise persistent runtime connectors

Status: implemented

## Problem

Remote Control onboarding wrote a persistent connector config and launched one detached `choruz-connector` process. `choruz-server` also launched saved configs once during startup. Neither process observed a connector after launch, so an ordinary connector crash left the Company device offline until the whole Choruz host restarted. The two launch paths could also race and rely on the connector's file lock to reject one of them.

## Decision

The API Gateway owns one `ConnectorSupervisor` from `crates/choruz-supervisor/src/connectors.rs` for its full process lifetime. The supervisor scans the configured connector directory (default `~/.choruz/connectors/`) every 500 milliseconds, starts each JSON config, reaps unexpected exits, and restarts failures with delays capped at 30 seconds. A process that remains alive for 60 seconds resets its failure count. Adding a config starts a connector without restarting Choruz; removing a config stops and reaps that connector.

The existing per-config lock remains the process-level uniqueness boundary. The supervisor checks it before spawning, so a connector left alive by an older release keeps running without duplicate restart churn; supervision takes ownership after that process releases the lock. Remote Control onboarding only performs the finite `pair-relay` exchange and installs the config. It does not create a second detached runtime process.

Independent API instances can select an absolute `CHORUZ_CONNECTOR_CONFIG_DIR`; onboarding and supervision resolve it through one function. The default remains the user's connector directory. Per-config locks prevent simultaneous processes, but cannot decide which unrelated instance should take ownership after a crash. Separate directories preserve that ownership boundary without changing the user's home or Harness credentials. Invalid explicit paths do not fall back to the shared default.

The API Gateway owns supervision rather than `choruz-server` because the gateway also runs directly in development and production service layouts. When `choruz-server` is the host entry point, its existing backend ownership transitively owns the gateway and all supervised connectors.

## Alternatives considered

**Keep a detached launch in onboarding and add a restart only to `choruz-server`.** Rejected because two owners would remain, and a standalone API Gateway would still lack recovery.

**Install one systemd or launchd unit per connector.** Rejected because onboarding must work without privileged service installation and across supported host layouts. An operator may still place the whole Choruz host under an operating-system service manager.

**Restart immediately without a cap.** Rejected because a corrupt config, revoked credential, or missing binary would create a hot crash loop. Bounded exponential backoff preserves recovery without consuming a core or flooding logs.

## Consequences

A paired device reconnects after its connector crashes without another credential or host restart. Connector lifetime is deliberately bounded by the API Gateway: graceful gateway shutdown stops and reaps its connector children. A crash of the entire detached `choruz-server` process still requires an operating-system service manager to restart the host; this decision repairs connector self-healing inside a running Choruz installation rather than installing host-level boot persistence.

The regression test uses a private temporary directory and executable, adds a config after the supervisor starts, observes a forced first-process crash and second start, removes the config, and waits for the child to acknowledge termination. It allocates no port or shared database state.
