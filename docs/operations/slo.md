# Choruz SLO / SLA Baseline

## Service Level Objectives

| Surface | SLI | Target |
|---------|-----|--------|
| API availability | `/healthz` success rate | 99.9% monthly |
| API readiness | `/readyz` success rate | 99.9% monthly |
| Message write latency | `SendMessage` P95 | < 150ms |
| Reconnect recovery | time to resume from cursor | < 3s |
| Webhook retry success | pending delivery recovery | < 5m |
| Audit completeness | privileged writes with audit row | 100% |

## Error Budgets

- Availability budget: 43m 12s per 30-day window (0.1% of 30 days)
- Message latency budget: 5% of calls may exceed 150ms, none may exceed 1s without paging
- Backlog budget: `choruz_event_backlog_total` should stay below 500 for steady-state production traffic

## Paging Rules

- Page immediately if `choruz-api-gateway` is down for more than 2 minutes
- Page immediately if `/readyz` fails for more than 2 minutes, even when `/healthz` still succeeds
- Page immediately if message write latency exceeds 500ms P95 for 10 minutes
- Open a ticket if webhook backlog stays above threshold for more than 5 minutes
- Open a ticket if audit throughput drops to zero during business hours

## Host-Native Metrics Map

- `choruz_principals_total`
- `choruz_conversations_total`
- `choruz_messages_total`
- `choruz_audit_logs_total`
- `choruz_event_backlog_total`
- `choruz_realtime_gateway_up`
- `choruz_agent_gateway_up`

## Release Gate

Do not promote a release unless the following are true:

1. All automated tests pass.
2. Alert rules load cleanly.
3. The current release package is below the binary size guardrail.
4. Rollback to the previous release has been rehearsed.
