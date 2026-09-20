# Choruz Host-Native Runbook

## Scope

This runbook covers the host-native `non_docker` deployment:

- `choruz-api-gateway`
- `choruz-pipeline` (the WebSocket/realtime pipeline)
- Caddy/Nginx reverse proxy
- local PostgreSQL, Redis, NATS, MinIO

## Standard Checks

1. `pnpm host:status`
2. `pnpm api:smoke`
3. `curl -fsS http://127.0.0.1:3000/healthz`
4. `curl -fsS http://127.0.0.1:3000/readyz`
5. `curl -fsS http://127.0.0.1:3020/readyz`
6. `curl -fsS http://127.0.0.1:3000/metrics`
7. Inspect `infra/host/.env` for drift

## Trace a reported interaction

Use [the activity CLI](cli.md#activity-data) to export observations for the affected time range and trace identifier. Join request logs by `trace_id` and `request_id`, then committed messages by their trace, turn and command identifiers. Client observations are not server audit evidence; missing terminal completion can mean a disconnected process, not a user decision. Remote operations are recorded on the host serving each request; this is not a cross-installation data warehouse.

Set `CHORUZ_LOG_FORMAT=json` for structured service stderr and use the process supervisor's log rotation and retention. `/metrics` includes `choruz_http_responses_total{class="5xx"}` for server error rate and `choruz_activity_batches_total{outcome="failed"}` for authenticated ingestion failures. These are process counters, not deduplicated event totals; a retried committed batch increments the successful-attempt counter again. Persisted records are the source for behaviour analysis.

## Behavior community publisher

Public contribution requires `CHORUZ_COMMUNITY_HF_TOKEN` in the API gateway's service environment. Use a Hugging Face publisher identity authorized to propose changes to `gjcjcg/ai-bad-behavior-library`; protect the token with the service's secret manager, not a browser field or trace. An absent token leaves local learning and public reads available and displays the missing publisher configuration in the learning panel. Contribution still requires each learning owner's separate permission.

Check the panel's per-record state and community synchronization error. A blocked preparation can be retried after resolving the analyst or privacy failure. An uncertain contribution may already have reached Hugging Face: inspect the dataset's discussions before taking any manual action. Acceptance is recognized only when the exact reviewed payload appears in an accepted dataset revision. A pending label means acceptance has not been observed, not that a reviewer is necessarily still working on it.

## Incident: API Gateway Down

1. Check `launchctl print system/com.choruz.api-gateway` on macOS or `systemctl status choruz-api-gateway` on Linux.
2. Review `/Users/Shared/choruz/logs/api-gateway.log` or `/var/log/choruz/api-gateway.log`.
3. If the binary is missing or stale, activate a verified package using [the deployment procedure](deploy.md#managed-device-upgrades).
4. If the newest release is bad, use that procedure's verified rollback with the installation's readiness URLs.

## Incident: Headless host startup or child failure

`choruz-server` starts embedded PostgreSQL and supervises the API gateway and pipeline. This is distinct from the externally managed database deployment in [deploy.md](deploy.md#ci-artifacts-and-publication). Keep the complete server stderr from the start attempt: child stdout and stderr join that stream, while `CHORUZ_LISTENING=3000` on stdout is the readiness handshake. That line is emitted only after both backend services pass their versioned readiness checks; a root-page or process-exists check is insufficient.

Use the first failure in the stream, not only the final shutdown message:

| Log pattern | Meaning and next action |
| --- | --- |
| `no migrations dir found (neither beside binary nor in workspace)` | The executable cannot find the release's SQL directory. A bundle requires `migrations/` beside `choruz-server`, `choruz-api-gateway` and `choruz-pipeline`; a source build resolves the ancestor containing both `Cargo.toml` and `apps/`, then its `migrations/`. Restore the complete matching release, not just the server binary. Changing the shell's working directory does not change this executable-relative search. |
| `migrations dir` | The `migrations_dir` field identifies the selected directory. The bundle location takes precedence over the source location. Check it belongs to the same release as the binaries before diagnosing a migration error. |
| `embedded postgres failed to start` | Read the `error` field: `pg setup`, `pg start`, database creation and migration errors name different stages. Correlate `embedded postgres booting`'s `data_dir` with the service account's writable data directory. Check disk space, permissions, first-install download access and port 5433 ownership. Do not delete `pgdata`, reset migrations or replace system libraries as a generic recovery step. |
| `backend spawn failed` | Read the named child and readiness error. Restore missing sibling binaries from the same release. For a readiness timeout, inspect that child's preceding stderr and database connectivity. Ports 3000 and 3020 must be free or occupied by the expected compatible service; do not kill an unrelated listener. |
| `backend child exited after startup` | The `error` field identifies the child and exit status, or the failure inspecting it. The supervisor stops the other children it owns. Preserve their preceding logs before the service manager retries the host. |
| `backend child failed; terminating choruz-server` | This is the terminal consequence, not the root cause. The host exits unsuccessfully after stopping owned children and PostgreSQL. Find the preceding child-exit record, fix its cause, then restart the complete host. |

Embedded PostgreSQL keeps its data under the service user's OS data directory, in `choruz/pgdata`; downloaded executables live in `choruz/pg-install`. Its command timeout is 60 seconds by default; `CHORUZ_POSTGRES_COMMAND_TIMEOUT_SECS` accepts a positive number of seconds. Increase it only when logs establish that an otherwise healthy storage operation exceeds the deadline. A missing executable interpreter or incompatible library is not a timeout.

Preserve database backups and the exact failed release before recovery. Restore missing migration files from that release; do not edit applied SQL or remove migration tracking rows. A binary rollback does not reverse a schema change; follow [managed device upgrades](deploy.md#managed-device-upgrades) for compatibility checks and verified rollback.

### Alerting and verification

Configure the deployment's existing log collector or service manager to alert on startup failure, unexpected child exit and repeated host restarts. Include the host identity, service, timestamp, exit status and a link to its retained stderr. Do not forward full process environments, connection strings or account files into notifications. Choruz emits the failure signals; it does not provision a paging service or log retention for the operator.

Check both versioned readiness endpoints after recovery, not just the API's liveness. A missing scrape or failed readiness probe is a host-availability incident even when no error counter is available: a process that never starts cannot serve metrics. Collect the pipeline's own metrics endpoint on port 3020 separately from the gateway's port 3000. Keep alerts scoped to the actual configured ports for managed deployments; the headless supervisor uses its fixed backend ports.

## Incident: Event Backlog Growing

1. Query `/metrics` and inspect `choruz_event_backlog_total`.
2. Verify webhook targets are returning `2xx`.
3. Call `POST /v1/webhooks/flush` to retry pending deliveries.
4. If backlog still grows, restart `choruz-api-gateway` and verify clients can resume from cursor.

## Incident: Secret Rotation Failed

1. Confirm the audit trail contains `agent.secret_rotated`.
2. Verify no token or secret leaked to logs.
3. Rotate again from the signed-in human principal.
4. If clients are wedged, disable the principal and create a replacement agent.

## Release Checklist

1. `cargo test --workspace`
2. `pnpm web:check`
3. `pnpm web:build`
4. `pnpm host:smoke`
5. `pnpm api:smoke`
6. `infra/ops/check.sh`
7. Activate the verified CI artifact using [the deployment procedure](deploy.md#managed-device-upgrades).

## Rollback Checklist

1. Confirm the current failure is release-induced rather than dependency-induced.
2. Run the [verified rollback](deploy.md#managed-device-upgrades) with explicit service readiness URLs.
3. Re-run `pnpm api:smoke`.
4. Verify `/metrics` and the alert panel return to green.
