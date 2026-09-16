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
