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

## Activity data

Use `choruz activity` with the same authentication as other CLI commands. Reads contain only the signed-in human's records in their home workspace and current companies. They do not grant access to other people's activity. The API's filters and pagination are defined in [the HTTP contract](../../openapi/choruz.yaml); the data stays on the API host being queried.

`activity list`, `activity export` and `activity summary` require `--since` and `--until` as RFC3339 timestamps, at most 31 days apart. `--source telemetry` selects browser observations; `--source audit` selects projected server audit records. `--trace-id` narrows the range to a request chain. Time filters use server receipt time, not the client clock. Each exported observation also carries its occurrence time when supplied by the browser.

`list` prints one JSON page with an opaque continuation cursor. `export` follows every page and prints one JSON object per line; redirect stdout to a protected file for analysis. A failed request or output write exits nonzero, and an interrupted export can leave a partial file. `summary` reports event counts, explicitly failed outcomes and mean recorded durations by name. Its `truncated` field identifies more than 200 groups; export the records when that happens. Counts are not unique users, intent, satisfaction or inferred abandonment.

`choruz activity messages --conversation <id>` exports committed message metadata and execution attempts as JSON lines. Add `--include-content` to include committed conversation text; membership is checked and reads are audited. This message projection excludes native CLI transcript text, terminal keystrokes, unsent drafts and tool payloads. Browser telemetry separately includes bounded component-input snapshots, including unsent ordinary drafts, under the [web activity contract](../subsystems/web-client.md#data). Treat those exports as private user data. Terminal activity describes transport operations, not model success.

`choruz activity prune --before <RFC3339>` previews at most 1000 old telemetry rows. Add `--apply` to delete that batch; repeat only while `has_more` is true. The cutoff must be at least 24 hours old. Deletion and its audit marker commit together. This command never deletes audit logs, message history or other users' rows. There is no automatic retention schedule; an operator can schedule the explicit command under their own policy. Exported files have their own retention and access controls.

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

For independent host instances under one operating-system user, set `CHORUZ_CONNECTOR_CONFIG_DIR` to a different absolute directory for each API process. Onboarding writes and supervision reads that same directory. An unset value uses `~/.choruz/connectors`; an empty or relative value is rejected rather than falling back. This isolates connector ownership only: each instance also needs separate database, runtime and listener configuration. It does not change Harness login stores or move existing connector configurations.
