# Agent Note: One shared Prometheus registry for the gateway and the pipeline

Status: implemented

## Problem

A `/metrics` handler that assembles its Prometheus text by hand couples every
metric to the endpoint. In the gateway that shape is a `format!` block for the
application gauges, `choruz_http_requests_total` and the latency buckets, plus
a helper appending the channel-task counters, which are `AtomicU64` statics in
`crates/choruz-application/src/db_service/group_workflow_tasks.rs` copied into a
`ChannelTaskMutationCounters` struct so the handler can print them; the HTTP
counter and latency buckets are `Arc`s threaded through `ApiState` and a
`MiddlewareState`. Adding a metric touches the feature, the application
types, the state structs and the handler's text, the hand-written `# TYPE`
line declares the cumulative latency buckets as a `gauge`, and the pipeline,
which has no such handler, exposes no metrics at all.

## Decision

`crates/choruz-common/src/metrics.rs` owns one process-wide `prometheus::Registry`
behind a `LazyLock`. `register_counter`, `register_counter_vec`,
`register_gauge` and `register_histogram` create a metric and register it;
`text()` encodes the registry with `TextEncoder`; `TEXT_CONTENT_TYPE` is
`text/plain; version=0.0.4`. A feature declares its metric once as a
`static X: LazyLock<IntCounter>` and increments it where the event happens.
Registration panics on a duplicate or invalid name: every metric is registered
exactly once from a static, so a duplicate is a programming error surfaced on
first use. A metric that must report `0` before its first event is forced at
startup by its owner; `DbService::new` forces the four
`choruz_channel_task_*_total` counters.

The gateway's `/metrics` handler in `services/choruz-api-gateway/src/meta_handlers.rs`
sets the `choruz_principals_total`, `choruz_conversations_total`,
`choruz_messages_total`, `choruz_audit_logs_total` and
`choruz_event_backlog_total` gauges from `ChatApp::metrics_snapshot` and
returns `common::metrics::text()`. The request middleware increments
`choruz_http_requests_total` and observes `choruz_http_request_duration`, a
histogram with buckets 0.05, 0.2 and 1 second, whose encoded samples keep the
`choruz_http_request_duration_bucket{le="…"}` names. `ApiState` carries no
metric state and the middleware takes none. The pipeline serves `GET /metrics`
from `services/choruz-pipeline/src/meta.rs` on its health port with the same
`text()`; each binary encodes its own registry.

The gauge refresh and encode run under a `Mutex` in the handler so a scrape
never returns another router's refresh: the registry is process-wide and the
gateway test binary drives several routers, each with its own `ChatApp`.

## Alternatives considered

**Keep a hand-rolled registry in `crates/choruz-common`.** A `Mutex<Vec<…>>` of
name, help, type and a boxed value reader would avoid the dependency, but it
would reimplement the exposition format, histogram bucket accounting, label
escaping and name validation that the `prometheus` crate already gets right,
and every future metric type would be more code to own.

**OpenTelemetry metrics with a Prometheus exporter.** `opentelemetry` and
`opentelemetry-prometheus` would give one API for traces and metrics, but the
system exports nothing to a collector, the exporter pulls in the SDK, resource
and view machinery, and the OTLP-style names would not match the
`choruz_*_total` names the alert rules in `infra/ops/alerts/choruz-alerts.yaml`
and `docs/operations/slo.md` already use.

**Register into the registry from the handler instead of from feature
statics.** Keeping registration next to the handler would preserve one
listing of every metric, which is the coupling the change removes; a feature
that adds a counter should not edit the endpoint.

**Keep the latency buckets as four counters printed as a gauge.** That
reproduces the hand-written text byte for byte, but it misdeclares a histogram
and omits `_sum` and `_count`; the histogram keeps the `_bucket` sample names
consumers query and emits the correct `# TYPE`.

## Consequences

A metric is one static and one `inc()` call, visible on `/metrics` in whichever
binary links the crate that declares it. The `prometheus` crate (with its
default `protobuf` feature) is a new dependency of `crates/choruz-common`, so every
crate carries it. Metrics are process-global, which is what a scrape wants, but
it means the gateway integration tests share counters across routers; tests
assert monotonic growth and per-line presence rather than absolute values, and
the scrape lock keeps each response's gauges consistent with the router that
served it. A dashboard matching the one-second bucket must use `le="1"`: the
encoder formats bucket bounds as shortest round-trip floats, and the `# TYPE`
line names the histogram, `choruz_http_request_duration`, not its `_bucket`
samples.

## Testing

`crates/choruz-common/src/metrics.rs` pins that a registered counter, gauge and
histogram appear in `text()` with their `# TYPE` lines and that a duplicate
registration panics. `metrics_endpoint_reports_prometheus_text` in
`services/choruz-api-gateway/src/tests/observability.rs` pins the content type, every
metric name, its `# TYPE` line and the latency bucket labels;
`metrics_endpoint_reports_channel_task_mutation_counters` in
`tests/channel_tasks.rs` pins that mutations move the channel-task counters.
`metrics_serves_the_shared_registry_as_prometheus_text` in
`services/choruz-pipeline/src/meta.rs` pins the pipeline endpoint.

## Related

- [Channel tasks board](../feature/2026-05-29-channel-tasks-board.md) owns the
  channel-task counters this registry serves.
