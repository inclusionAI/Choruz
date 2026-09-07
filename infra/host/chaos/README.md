# Fault-recovery evidence

Run `bash infra/host/recovery_smoke.sh` for owned process-loss and database-network
recovery acceptance. It starts a disposable PostgreSQL, API and pipeline and
substitutes only the external CLI. The CI DB/API smoke job runs it too.

The process-loss case kills its own CLI child, then verifies the same command
retries and reaches committed state with exactly one expected reply visible
through the message API. The network case disconnects a private TCP proxy,
observes failed pipeline readiness, accepts a message through the direct API,
restores the proxy and verifies committed delivery. Command IDs and attempt
counts are printed as evidence. This is not live model-service acceptance.

The unsafe global process-kill and firewall scripts are retired. They selected
unrelated Claude processes or changed shared host networking; their verifiers
could exit successfully without observing recovery. Do not restore them as CI
gates or use their old output as acceptance evidence.

Existing focused owners cover narrower contracts:

- `services/choruz-pipeline/src/executor/tests.rs` checks actual owned CLI process
  termination, failure classification, stale-session retry and WAL recovery.
  External CLI responses are fixtures, not live model-service acceptance.
- `infra/host/tests/process_lifecycle.test.sh` checks process-record ownership
  before stopping its own disposable child.
- `infra/host/migration_smoke.sh` checks real disposable PostgreSQL migrations and
  notification delivery, not recovery from a network partition.

A different manual network-fault experiment needs its own disposable stack and an explicit
proxy endpoint, not system firewall changes. Record the affected command ID,
pre-fault state, injected fault, recovery transition and final persisted message.
Missing any of these leaves end-to-end recovery unverified, regardless of health
checks. Stop only experiment-owned processes and preserve the evidence log.
