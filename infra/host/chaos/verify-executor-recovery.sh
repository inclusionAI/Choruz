#!/usr/bin/env bash
# verify-executor-recovery.sh — Verify that the system recovered from
# a killed executor process.
#
# Checks:
#   1. The pipeline process is still running
#   2. The lease monitor has detected the expired lease
#   3. The command has been retried or dead-lettered
#   4. A new executor process has been spawned
set -euo pipefail

echo "=== Verify: executor recovery ==="

# 1. Check that the pipeline is still running
if pgrep -f 'choruz-pipeline' > /dev/null; then
    echo "[PASS] choruz-pipeline process is running"
else
    echo "[FAIL] choruz-pipeline process is NOT running"
    exit 1
fi

# 2. Check for recovered executor processes
EXECUTOR_COUNT=$(pgrep -fc 'claude.*--output-format.*stream-json' || echo "0")
echo "[INFO] Active executor processes: $EXECUTOR_COUNT"

# 3. Check health endpoint
HEALTH_URL="${CHORUZ_PIPELINE_HEALTH_URL:-http://127.0.0.1:3020/readyz}"
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$HEALTH_URL" 2>/dev/null || echo "000")
if [[ "$HTTP_CODE" == "200" ]]; then
    echo "[PASS] Health endpoint returned 200"
else
    echo "[WARN] Health endpoint returned $HTTP_CODE (may still be recovering)"
fi

# 4. Check metrics for retry activity
METRICS_URL="${CHORUZ_PIPELINE_METRICS_URL:-http://127.0.0.1:3020/metrics}"
METRICS=$(curl -s "$METRICS_URL" 2>/dev/null || echo "")
if echo "$METRICS" | grep -q "command_dispatch"; then
    echo "[PASS] Metrics endpoint is responsive"
else
    echo "[WARN] Metrics endpoint not responding or no dispatch metrics yet"
fi

echo ""
echo "Manual verification steps:"
echo "  1. Check pipeline logs for 'lease expired' messages"
echo "  2. Check pipeline logs for 'retry scheduler' activity"
echo "  3. Check pipeline logs for 'persistent claude session spawned'"
echo "  4. Send a test message and verify it gets a response"
