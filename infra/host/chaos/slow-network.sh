#!/usr/bin/env bash
# slow-network.sh — Simulate network latency on database connections.
#
# Uses macOS dnctl/dummynet or Linux tc to add configurable delay to
# TCP traffic on the PostgreSQL port.
#
# Usage:
#   ./infra/host/chaos/slow-network.sh [delay_ms] [duration_seconds]
#   Default: 200ms delay for 30 seconds
#
# WARNING: Requires sudo.
set -euo pipefail

DELAY_MS="${1:-200}"
DURATION="${2:-30}"
PG_PORT="${CHORUZ_PG_PORT:-5432}"

echo "=== Chaos: slow-network ==="
echo "Adding ${DELAY_MS}ms latency to port ${PG_PORT} for ${DURATION} seconds"

OS=$(uname -s)

add_latency() {
    if [[ "$OS" == "Darwin" ]]; then
        # macOS: use dnctl (dummynet)
        sudo dnctl pipe 1 config delay "${DELAY_MS}ms" 2>/dev/null || {
            echo "WARNING: dnctl not available. Install or enable dummynet."
            echo "Falling back to no-op simulation."
            return 1
        }
        echo "dummynet pipe 1 from any to any dst-port ${PG_PORT} pipe 1" | \
            sudo pfctl -a choruz-chaos-slow -f - 2>/dev/null
        sudo pfctl -e 2>/dev/null || true
        echo "Latency applied (macOS dummynet)"
    else
        # Linux: use tc
        local IFACE
        IFACE=$(ip route | grep default | head -1 | awk '{print $5}')
        sudo tc qdisc add dev "$IFACE" root netem delay "${DELAY_MS}ms" 2>/dev/null || {
            echo "WARNING: tc/netem not available."
            return 1
        }
        echo "Latency applied (Linux tc/netem on $IFACE)"
    fi
}

remove_latency() {
    if [[ "$OS" == "Darwin" ]]; then
        sudo pfctl -a choruz-chaos-slow -F all 2>/dev/null || true
        sudo dnctl pipe 1 delete 2>/dev/null || true
        echo "Latency removed (macOS)"
    else
        local IFACE
        IFACE=$(ip route | grep default | head -1 | awk '{print $5}' 2>/dev/null || echo "eth0")
        sudo tc qdisc del dev "$IFACE" root netem 2>/dev/null || true
        echo "Latency removed (Linux)"
    fi
}

trap remove_latency EXIT

if add_latency; then
    echo "Waiting ${DURATION} seconds..."
    sleep "$DURATION"
    remove_latency
    trap - EXIT
else
    trap - EXIT
    echo ""
    echo "Could not apply network latency. To test manually:"
    echo "  1. Use a tool like 'toxiproxy' or 'comcast'"
    echo "  2. Or run: sudo tc qdisc add dev lo root netem delay ${DELAY_MS}ms"
fi

echo ""
echo "Expected behavior during latency injection:"
echo "  - Database operations take ~${DELAY_MS}ms extra"
echo "  - Heartbeats may lag (but should not expire if DELAY_MS < timeout)"
echo "  - CDC poll cycle time increases proportionally"
echo "  - No crashes or data loss"
echo ""
echo "Verify with: ./infra/host/chaos/verify-executor-recovery.sh"
