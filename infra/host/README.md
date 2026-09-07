# Host-Native Bootstrap

This directory contains scripts for running the single-host Choruz baseline directly on the host machine.

Required services:

- PostgreSQL
- local filesystem directories for attachments and backups

The scripts assume macOS with Homebrew by default, but the environment variables can be overridden for manual installations.

Local development keeps the standard ports when they are free. If any default port is occupied, a normal checkout or Git worktree records a stable available set in `infra/host/.env` before the stack starts. Explicit `CHORUZ_*_PORT` values and `CHORUZ_ENV=production` disable automatic allocation and fail normally if unavailable.

The API, backup, performance and web E2E smoke entry points require Node and create private configurations, ports and PostgreSQL/data directories in a temporary `choruz-smoke-*` directory under the user's home, within the application's default browse roots. They pass an absolute runtime path so the Web and API processes resolve the same workspaces. They do not use the development `.env` or inherited database URLs. Cleanup removes the owned data after PostgreSQL stops; logs remain under `.choruz-runtime/logs/choruz-smoke-*`. `CHORUZ_HOST_ENV_FILE` selects the generated configuration for child host scripts and fails if the file is missing.

Available entrypoints:

- `start.sh` / `stop.sh` / `status.sh`: start and stop PostgreSQL and report the managed data directories
- `smoke.sh`: verify the PostgreSQL port and required filesystem paths are ready
- `api_smoke.sh`: boot `choruz-api-gateway` and probe `/healthz` plus `/metrics`
- `migrate.sh`: apply or reset SQL migrations against a host-native PostgreSQL database
- `migration_smoke.sh`: create a throwaway database, apply migrations twice, verify schema/indexes/UTF-8, reset, and clean up
- `perf_ws_smoke.sh`: boot `choruz-api-gateway`, install `k6` if needed, and run the WebSocket backlog smoke baseline
- `backup_smoke.sh`: exercise backup and restore against a seeded local dataset
- `dev.sh` / `dev_stop.sh` / `dev_reload.sh`: start, stop and reload `choruz-api-gateway`, `choruz-pipeline` and the pipeline watchdog for local development (`pnpm dev:all`, `pnpm stop:all`, `pnpm reload:local`)
- `web_dev.sh`: start the Next.js dev server against the local gateway (`pnpm dev:web`)
- `pipeline_watchdog.sh`: restart `choruz-pipeline` when its readiness probe fails; spawned by `dev.sh`
- `web_e2e.sh`: build and start the full stack, then run the Playwright specs (`pnpm web:e2e`)
- `setup_test_database.sh`: create the shared PostgreSQL test database that cargo integration tests use; CI sources it
- `runtime_conversion_rehearsal.sh`: rehearse the offline `.choruz-runtime` conversion (see `docs/testing/choruz-runtime-conversion-rehearsal.md`)
- `smoke/`: real-driver smokes that need a Claude or Codex binary (`pnpm smoke:real-harness`)
- `recovery_smoke.sh`: owned CLI process-loss and database-proxy recovery with final delivery assertions (see `chaos/README.md`)
- `perf/`: the k6 WebSocket baseline and its installer, driven by `perf_ws_smoke.sh`
- `tests/`: shell tests for these scripts; `process_lifecycle.test.sh` includes HTTP readiness checks and runs in CI as the host lifecycle policy test
