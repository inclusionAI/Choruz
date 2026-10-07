# choruz-router

Route conversation events to agent commands with the platform's mention, coordinator and task policies. `route_event` reads members through `MemberProvider` and records visibility, decisions and commands through `DecisionSink`. It builds the same `[choruz-incoming]` envelope for any harness; execution is the caller's responsibility.

## Standalone use

Set `default-features = false` on the Cargo dependency to use routing without PostgreSQL. The default `postgres` feature adds `run_router_loop`, which consumes the durable store's CDC channel. The pipeline uses this feature and supplies the PostgreSQL implementations of both traits.

```sh
cargo run -p choruz-router --no-default-features --example route_message
```

The example prints a command for the mentioned reviewer, not the unmentioned researcher. It uses the existing in-memory adapters and does not start an agent or save state. A host application must supply trusted membership, authorization and a durable, idempotent sink before using this as a message service. Disabling the database adapter is not a substitute for those guarantees.

## Entry points

- `src/router.rs` — `route_event`, `run_router_loop`, `MemberProvider`, `DecisionSink`, prompt building
- `src/policy.rs` — `evaluate_trigger`
- `src/workflow.rs` — `parse_workflow_routing_event`
- `src/models.rs` — the `mailbox_visibility`, `route_decisions` and `conversation_members` row types

## Tests

`cargo test -p choruz-router` covers in-memory providers and PostgreSQL routing. Source `infra/host/setup_test_database.sh` first to supply the required `CHORUZ_TEST_DATABASE_URL`; database tests fail when it is missing.

`cargo test -p choruz-router --no-default-features` runs the same routing and policy tests without the PostgreSQL loop. CI checks this build separately from the default-feature tests to prevent Cargo feature unification from hiding a database dependency.

## Related

- [docs/subsystems/message-pipeline.md](../../docs/subsystems/message-pipeline.md) — the router stage and its neighbours
- [docs/subsystems/agent-protocol.md](../../docs/subsystems/agent-protocol.md) — the `[choruz-incoming]` envelope the router writes
- [docs/architecture.md](../../docs/architecture.md)
