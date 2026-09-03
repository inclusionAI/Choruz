# Chaos Engineering Scripts

Scripts for testing system resilience.

## Scripts

| Script | Purpose | Verifier |
|--------|---------|----------|
| `kill-executor.sh` | Kill random executor process (tests heartbeat timeout + retry) | `verify-executor-recovery.sh` |
| `partition-db.sh` | Block DB traffic (tests graceful degradation) | `verify-db-recovery.sh` |
| `slow-network.sh` | Add latency to DB connections (tests timeout handling) | `verify-executor-recovery.sh` |

## Quick Start

```bash
# 1. Ensure choruz-pipeline is running
# 2. Run a chaos scenario
./infra/host/chaos/kill-executor.sh

# 3. Wait for recovery (lease_timeout_secs, default 60s)
sleep 65

# 4. Verify
./infra/host/chaos/verify-executor-recovery.sh
```

## Prerequisites

- `kill-executor.sh`: No special requirements
- `partition-db.sh`: Requires sudo (uses pf on macOS, iptables on Linux)
- `slow-network.sh`: Requires sudo (uses dummynet on macOS, tc/netem on Linux)
