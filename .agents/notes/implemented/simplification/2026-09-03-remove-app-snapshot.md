# Agent Note: Use PostgreSQL as the only durable application state

Status: implemented

## Problem

The API Gateway could start from an `app_snapshot` row after a database load failure even though reads and writes already depended on PostgreSQL. That snapshot omitted messages and audit logs, so the fallback could present stale or partial principals, conversations and webhook configuration without making the service usable independently of the database.

## Decision

PostgreSQL is the API Gateway's only durable source of truth. The gateway verifies database connectivity and loads its process-local `ChatApp` shell from PostgreSQL before binding its listener. A connection or load failure stops startup.

`V038__remove_app_snapshot.sql` drops the obsolete table. The frozen `0002_app_snapshot.sql` migration remains in history so existing databases and fresh installs traverse the same append-only migration chain.

## Alternatives considered

**Keep the snapshot as a startup fallback.** Rejected because the snapshot does not contain all state required by the gateway and PostgreSQL remains mandatory for request handling.

**Keep writing a shutdown snapshot only for recovery tooling.** Rejected because no recovery tool consumes it, and retaining the serializer and table would preserve a second durability contract with no safe runtime use.

## Consequences

Startup fails clearly when PostgreSQL is unavailable instead of serving partial state. The snapshot store, shutdown flush, configuration switch, CBOR dependency and table disappear. Recovery and backup procedures must operate on PostgreSQL.
