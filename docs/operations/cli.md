# Choruz CLI

`choruz` is the scriptable control-plane client for a running Choruz host.
It uses the same authenticated HTTP API as the Web Dashboard; it never writes
the database directly.

## Install

Release bundles include the `choruz` binary. From a source checkout, build it
with:

```bash
cargo build --release -p choruz-cli
./target/release/choruz --help
```

## Commands available now

```bash
choruz status
choruz company list
choruz agent list
choruz remote status
choruz remote pairing-credential
```

Use `--json` for automation. `--api-url` and `--pipeline-url` select another
host; they default to `CHORUZ_API_BASE_URL` and `CHORUZ_PIPELINE_URL`.

Authenticated commands accept `CHORUZ_SESSION_TOKEN`. On the host itself,
where the API URL is loopback, the CLI may instead use
`CHORUZ_OPERATOR_USER` and `CHORUZ_OPERATOR_PASSWORD` to obtain a short-lived
session token. Supplying the operator password to a remote URL is deliberately
not supported; use a session token there.

## Remote Control from a server

Remote Control uses Choruz's hosted Gateway by default. On a fresh server,
run:

```bash
choruz start
```

This starts the bundled headless host as a detached background process, waits
until the local API and its Cloud Gateway pairing socket are ready, and then
prints a single-use, five-minute pairing credential. The command can exit
without stopping the host or invalidating the credential. Its process ID and
log are stored in the platform data directory under `choruz/`.

Paste the credential in the Remote Control Web Dashboard to pair that browser
to the server's Choruz installation. Agent processes and files remain on the
server. A later `choruz start` reuses the running host and issues a new
credential. `choruz remote pairing-credential` does the same when Choruz is
already running.

No Cloudflare account, Worker deployment, or shared Gateway secret is needed.
Set `CHORUZ_REMOTE_CONTROL_GATEWAY_URL` only to use a self-hosted Gateway; in
that advanced mode, set the matching `CHORUZ_REMOTE_CONTROL_GATEWAY_SECRET`.

`choruz-connector` is separate: it joins an execution machine to an existing
Choruz host. It does not replace `choruz` or host the Remote Control bridge.
