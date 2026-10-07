# Choruz computer use

Inspect, install and enable the device's BrowserSkill and CuaDriver tools without a Choruz server, Company or database. The platform dispatches to the same `manage` operation for local and remote devices.

```sh
cargo run -p choruz-computer-use --example status
```

The example runs installed diagnostic commands without installing tools or changing consent. Missing tools produce `needs_attention` checks. `manage(Some(tool), Some(true))` explicitly installs or repairs the selected tool and its agent skill in the device user's home. Installation runs asynchronously in the caller's process; keep that process alive until the returned health state leaves `installing`. Disabling controls automatic profile exposure, not OS permissions or execution of user-installed binaries.

Browser extensions, an available graphical desktop and OS permissions remain external prerequisites. The caller supplies authorization and owns process lifetime. This package does not collect traces, create accounts, run a learning worker or distribute a Harness binary. Claude Code and Codex profiles discover device-installed skills through `choruz-agent-runtime`.
