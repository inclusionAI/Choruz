# Agent Note: OpenAPI as the one external contract

Status: implemented

Supersedes [contract-first external APIs](../../archived/architecture/2026-08-18-contract-first-external-apis.md).

## Problem

The repository carried three contract surfaces that nothing consumed. `proto/choruz/v1/chat.proto` described a gRPC service no crate implemented (no `tonic` or `prost` anywhere), `buf lint` ran on it in every Static checks job after downloading the `buf` binary, and `crates/choruz-proto` existed only to embed that file and `openapi/choruz.yaml` for two `contains` tests. The SDKs under `sdk/` (Rust, TypeScript, Python) wrapped 16 routes from the first API cut while the gateway registers 107; CI checked that they compiled and used no legacy names, never that they matched the API. Meanwhile the one real contract, `openapi/choruz.yaml`, was 33 routes behind the gateway and listed a route that does not exist, and nothing could tell. The "contracts first, and the SDKs in the same change" rule in `AGENTS.md` asked every API change to update artifacts no caller used. `crates/choruz-workspace`, the first worktree manager, had no dependents either: worktree provisioning lives in `crates/choruz-agent-runtime` and the gateway.

## Decision

`openapi/choruz.yaml` is the only external contract. The gateway test `openapi_documents_every_route` (`services/choruz-api-gateway/src/tests/contracts.rs`) reads the route table in `lib.rs` and the plugin routers, normalises path parameters, and fails on any path present on one side only, so the spec cannot drift again. The proto file, `buf.yaml`, `crates/choruz-proto`, `crates/choruz-workspace`, the three SDKs, `scripts/check-choruz-sdk-names.sh` and their CI steps are deleted; `deny.toml` keeps only the advisory ignore that still matches a crate in `Cargo.lock`. The "Contracts first" rule now names `openapi/` and the migration only.

## Alternatives considered

- **Keep the SDKs and bring them up to date**: rejected. Choruz has no external SDK users, the web client calls the gateway through its own Next.js routes, and three hand-written clients would cost every API change a triple update with no consumer to catch a mistake. When an SDK is wanted, generating it from `openapi/choruz.yaml` is the path, and the route-coverage test is what makes that spec trustworthy.
- **Keep `chat.proto` as a design document for a future gRPC surface**: rejected. A contract nobody implements is documentation that lies about the system; the OpenAPI file already records every route, and a gRPC decision would start from a new note.
- **Enforce spec coverage in a CI script instead of a Rust test**: rejected. The route table is Rust source; a `cargo test` in the gateway runs whenever the gateway changes, needs no extra tool on the runner, and fails on the developer's machine first.
- **Delete `openapi/` as well and let `lib.rs` be the contract**: rejected. The spec carries the route classes (`x-choruz-route-classes`) and operation ids that the docs reference, and a machine-readable inventory is what an SDK generator or an external integrator reads.

## Consequences

- Static checks no longer downloads `buf`; the job keeps trivy and cargo-deny.
- `openapi/choruz.yaml` gains the runtime-host, remote-control, thread and workspace-session routes it lacked, and loses `/v1/speech-to-text`.
- The four legacy workflow-task routes, which only answered 403 "use channel task APIs", are deleted with `handlers_workflow_tasks.rs` rather than documented.
- A new route without a spec entry fails `cargo test -p choruz-api-gateway`.
- `docs/testing/pr-test-policy.md` and the pre-push skill no longer list SDK or contract-lint jobs.
