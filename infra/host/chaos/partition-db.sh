#!/usr/bin/env bash
# partition-db.sh — Simulate a database connection interruption by
# temporarily blocking TCP traffic to the PostgreSQL port.
#
# Uses macOS pf (packet filter) or iptables on Linux.
#
# The pipeline should:
#   - Fail DB operations with connection errors
#   - Continue running (not crash)
#   - Recover automatically when connectivity is restored
#
# Usage:
#   ./infra/host/chaos/partition-db.sh [duration_seconds]
#   Default duration: 10 seconds
#
# WARNING: This modifies firewall rules. Requires sudo.
set -euo pipefail

DURATION="${1:-10}"
PG_PORT="${CHORUZ_PG_PORT:-5432}"
PG_HOST="${CHORUZ_PG_HOST:-127.0.0.1}"

echo "=== Chaos: partition-db ==="
echo "Blocking TCP to ${PG_HOST}:${PG_PORT} for ${DURATION} seconds"

OS=$(uname -s)

block_traffic() {
    if [[ "$OS" == "Darwin" ]]; then
        # macOS: use pf
        local RULE="block drop out proto tcp from any to ${PG_HOST} port ${PG_PORT}"
        echo "$RULE" | sudo pfctl -a choruz-chaos -f - 2>/dev/null
        sudo pfctl -e 2>/dev/null || true
        echo "Traffic blocked (macOS pf)"
    else
        # Linux: use iptables
        sudo iptables -A OUTPUT -p tcp --dport "$PG_PORT" -d "$PG_HOST" -j DROP
        echo "Traffic blocked (Linux iptables)"
    fi
}

unblock_traffic() {
    if [[ "$OS" == "Darwin" ]]; then
        sudo pfctl -a choruz-chaos -F all 2>/dev/null || true
        echo "Traffic unblocked (macOS pf)"
    else
        sudo iptables -D OUTPUT -p tcp --dport "$PG_PORT" -d "$PG_HOST" -j DROP 2>/dev/null || true
        echo "Traffic unblocked (Linux iptables)"
    fi
}

# Ensure cleanup on exit
trap unblock_traffic EXIT

block_traffic

echo "Waiting ${DURATION} seconds..."
sleep "$DURATION"

unblock_traffic
trap - EXIT

echo ""
echo "DB connectivity restored. Verify with:"
echo "  ./infra/host/chaos/verify-db-recovery.sh"
