# Deployment Runbook

> **Last updated:** 2026-03-29 | **Scope:** Host-native single-server deployment

---

## Prerequisites

- Access to the deployment host
- PostgreSQL 16+ running and accessible
- Claude Code and/or Codex CLI installed (for agent execution)
- Rust toolchain and pnpm available on the host
- Current working directory: `choruz/non_docker`

## Pre-Deploy Checks

```bash
# 1. Run CI gate — must pass before deploying
pnpm preflight

# 2. Check current service status
pnpm host:status

# 3. Verify database is accessible
./infra/host/smoke.sh
```

## Step-by-Step Deployment

### 1. Build Release Binaries

```bash
cargo build --release
```

Binaries are written to `target/release/`:
- `choruz-api-gateway` (~6.8 MB)
- `choruz-pipeline` (~5.8 MB)
- `choruz-replay` (~2.3 MB)

### 2. Build Web Frontend

```bash
pnpm --dir apps/web build
```

Output: `apps/web/.next/` (Next.js standalone build)

### 3. Apply Database Migrations

```bash
# Apply pending migrations
./infra/host/migrate.sh

# Verify migration integrity (optional but recommended)
./infra/host/migration_smoke.sh
```

### 4. Stop Running Services

```bash
./infra/host/dev_stop.sh
```

This stops:
- API Gateway
- choruz-pipeline
- Any active PTY agent sessions

### 5. Deploy Binaries

```bash
# If using the release script:
infra/ops/bin/release.sh deploy

# Or manually copy binaries to their expected locations
# (depends on your host setup — check infra/host/.env for paths)
```

### 6. Start Services

```bash
./infra/host/dev.sh
```

This starts:
- API Gateway on `:3000`
- choruz-pipeline on `:3020`

### 7. Start Web Frontend

```bash
pnpm --dir apps/web dev --port 3100
# Or for production:
# cd apps/web && node .next/standalone/server.js
```

### 8. Post-Deploy Verification

```bash
# Health check
curl -fsS http://127.0.0.1:3000/healthz

# Metrics endpoint
curl -fsS http://127.0.0.1:3000/metrics

# API smoke test
pnpm api:smoke

# Full smoke test
pnpm host:smoke

# Verify agent sessions resume
# Check that existing agents reconnect and respond to messages
```

## Rollback

If the deployment causes issues:

```bash
# 1. Stop current services
./infra/host/dev_stop.sh

# 2. Roll back to previous binaries
infra/ops/bin/rollback.sh

# 3. Restart
./infra/host/dev.sh

# 4. Verify
pnpm api:smoke
curl -fsS http://127.0.0.1:3000/metrics
```

See `docs/operations/runbook.md` for detailed incident response procedures.

## Post-Deploy Tasks

- [ ] Verify all agent bindings are in `idle` or `running` state (not `error` or `disabled`)
- [ ] Check that PTY sessions are reachable via the web UI
- [ ] Monitor `/metrics` for error rate spikes in the first 10 minutes
- [ ] If agents were disabled due to auth errors, re-enable them after confirming `secret_hash` is correct

## Environment Variables

Key variables that may need updating between deploys — see `docs/architecture.md` Appendix for the full list.

| Variable | Check |
|----------|-------|
| `CHORUZ_DATABASE_URL` | Matches current PostgreSQL instance |
| `CHORUZ_API_BASE_URL` | Correct host:port for the gateway |
| `CHORUZ_AGENT_TOKENS_FILE` | Points to valid token file |
| `RUST_LOG` | Appropriate for production (usually `info`) |
| `CHORUZ_LOG_FORMAT` | `human` for an interactive host or `json` for structured collection |

## Notes

- **No git remote** — deployments are from the local main branch
- **PTY sessions are lost on gateway restart** — agents will need to be re-triggered
- **Durability** — PostgreSQL is the only durable source of truth. The API Gateway refuses to start when it cannot load required state from the database
- **Agent tokens are hot-reloaded** — changes to `CHORUZ_AGENT_TOKENS_FILE` take effect without restart
