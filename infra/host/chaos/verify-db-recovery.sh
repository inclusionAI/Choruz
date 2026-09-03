#!/usr/bin/env bash
# verify-db-recovery.sh — Verify the system recovered from a DB partition.
set -euo pipefail

echo "=== Verify: DB recovery ==="

# 1. Pipeline still running
if pgrep -f 'choruz-pipeline' > /dev/null; then
    echo "[PASS] choruz-pipeline process is running"
else
    echo "[FAIL] choruz-pipeline process is NOT running — it crashed during partition"
    exit 1
fi

# 2. Health endpoint
HEALTH_URL="${CHORUZ_PIPELINE_HEALTH_URL:-http://127.0.0.1:3020/readyz}"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$HEALTH_URL" 2>/dev/null || echo "000")
if [[ "$HTTP_CODE" == "200" ]]; then
    echo "[PASS] Health endpoint returned 200"
else
    echo "[WARN] Health endpoint returned $HTTP_CODE"
fi

# 3. DB connectivity
PG_HOST="${CHORUZ_PG_HOST:-127.0.0.1}"
PG_PORT="${CHORUZ_PG_PORT:-5432}"
if nc -z "$PG_HOST" "$PG_PORT" 2>/dev/null; then
    echo "[PASS] PostgreSQL port ${PG_HOST}:${PG_PORT} is reachable"
else
    echo "[FAIL] PostgreSQL port ${PG_HOST}:${PG_PORT} is NOT reachable"
    exit 1
fi

echo ""
echo "Manual verification:"
echo "  1. Check pipeline logs for DB reconnection messages"
echo "  2. Send a test message and verify end-to-end delivery"
echo "  3. Check that CDC poller resumed processing outbox entries"
