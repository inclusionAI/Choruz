# Choruz desktop

The macOS application opens the shared task-first workbench and owns its local backend. It bundles the Next.js production server and Rust host binaries; it does not require a separately running development stack. Agent CLIs remain user-installed.

## Build and verify

On an Apple Silicon Mac with the repository's Node, pnpm and Rust prerequisites installed, run from the repository root:

```sh
pnpm desktop:prepare
pnpm --dir apps/desktop test
pnpm --dir apps/desktop test:bundle
pnpm desktop:package
```

The DMG and application are under `apps/desktop/dist/`. The runtime smoke check uses an isolated temporary data directory, authenticates through the real API, loads the workbench twice and verifies owned-service shutdown. Failed smoke checks retain their data and logs for diagnosis.

The package targets macOS arm64. Without a valid Developer ID configuration it is unsigned and not notarized; a local build is not a notarized public release. First launch needs network access to download the embedded PostgreSQL distribution.

## Application lifecycle

Closing the window keeps tasks running. The application menu reopens the window. Quitting asks for confirmation and stops the owned web server, backend and database. A second launch reuses the application instance.

Data and private credentials live in `~/Library/Application Support/Choruz Desktop`; this is separate from a CLI or development installation. `Open logs` opens that directory. Startup failures show an error and retain `host.log` and `web.log`. An occupied desktop port fails startup without terminating its owner. The fixed loopback ports are defined in [runtime.cjs](runtime.cjs) and compiled into the packaged web routes during preparation.

The native folder dialog is available only for the local workspace. Remote devices retain their device-side folder browser. The renderer is sandboxed without Node integration; its preload exposes only the sender-checked folder dialog. External HTTP links open in the system browser.
