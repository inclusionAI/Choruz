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

## Incident: API Gateway Down

1. Check `launchctl print system/com.choruz.api-gateway` on macOS or `systemctl status choruz-api-gateway` on Linux.
2. Review `/Users/Shared/choruz/logs/api-gateway.log` or `/var/log/choruz/api-gateway.log`.
3. If the binary is missing or stale, activate a verified package using [the deployment procedure](deploy.md#managed-device-upgrades).
4. If the newest release is bad, use that procedure's verified rollback with the installation's readiness URLs.

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
