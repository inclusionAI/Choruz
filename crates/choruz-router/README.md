# choruz-router

Router and policy engine of the message pipeline: `route_event` and `run_router_loop` consume outbox rows from `choruz-store`'s CDC channel, look up conversation members through a `MemberProvider`, evaluate each agent's trigger policy (`evaluate_trigger`), record mailbox visibility and route decisions through a `DecisionSink`, and build the `[choruz-incoming]` prompt for every agent command they emit. `services/choruz-pipeline` depends on it and supplies the PostgreSQL implementations of both traits.

## Entry points

- `src/router.rs` — `route_event`, `run_router_loop`, `MemberProvider`, `DecisionSink`, prompt building
- `src/policy.rs` — `evaluate_trigger`
- `src/workflow.rs` — `parse_workflow_routing_event`
- `src/models.rs` — the `mailbox_visibility`, `route_decisions` and `conversation_members` row types

## Tests

`cargo test -p choruz-router` runs against in-memory providers; the three tests in `src/router/tests.rs` that open PostgreSQL run only when `CHORUZ_DATABASE_URL` is set and skip otherwise.

## Related

- [docs/subsystems/message-pipeline.md](../../docs/subsystems/message-pipeline.md) — the router stage and its neighbours
- [docs/subsystems/agent-protocol.md](../../docs/subsystems/agent-protocol.md) — the `[choruz-incoming]` envelope the router writes
- [docs/architecture.md](../../docs/architecture.md)
