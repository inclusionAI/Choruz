#!/usr/bin/env bash
# kill-executor.sh — Kill a random executor (sandbox claude process) to test
# crash recovery.
#
# The Session Manager should detect the heartbeat timeout, bump the epoch,
# and schedule a retry.
#
# Usage:
#   ./infra/host/chaos/kill-executor.sh [--all]
#   --all   Kill ALL executor processes (default: kill one random process)
#
# What this exercises:
#   - Heartbeat timeout detection by the lease monitor
#   - Epoch bump + retry scheduling by the Session Manager
#   - WAL crash recovery on the next executor startup
set -euo pipefail

KILL_ALL=false
if [[ "${1:-}" == "--all" ]]; then
    KILL_ALL=true
fi

echo "=== Chaos: kill-executor ==="

# Find executor processes (persistent claude sessions spawned by choruz-pipeline)
PIDS=$(pgrep -f 'claude.*--output-format.*stream-json' || true)

if [[ -z "$PIDS" ]]; then
    echo "No executor processes found. Is choruz-pipeline running?"
    exit 0
fi

PID_ARRAY=($PIDS)
echo "Found ${#PID_ARRAY[@]} executor process(es): ${PID_ARRAY[*]}"

if [[ "$KILL_ALL" == "true" ]]; then
    echo "Killing ALL executor processes..."
    for pid in "${PID_ARRAY[@]}"; do
        echo "  kill -9 $pid"
        kill -9 "$pid" 2>/dev/null || true
    done
    echo "Killed ${#PID_ARRAY[@]} process(es)."
else
    # Pick a random process
    RANDOM_INDEX=$((RANDOM % ${#PID_ARRAY[@]}))
    TARGET_PID="${PID_ARRAY[$RANDOM_INDEX]}"
    echo "Killing random executor process: $TARGET_PID"
    kill -9 "$TARGET_PID" 2>/dev/null || true
    echo "Killed."
fi

echo ""
echo "Expected recovery behavior:"
echo "  1. Lease monitor detects heartbeat timeout (within lease_timeout_secs)"
echo "  2. Session Manager bumps epoch for affected session"
echo "  3. Command transitions to retry_scheduled"
echo "  4. Dispatch loop picks up the retried command"
echo "  5. New executor process spawned with fresh epoch"
echo ""
echo "Verify with: ./infra/host/chaos/verify-executor-recovery.sh"
