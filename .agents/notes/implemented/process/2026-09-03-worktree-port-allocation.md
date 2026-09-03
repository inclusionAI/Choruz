# Agent Note: Stable worktree port allocation

Status: implemented

## Problem

Local development defaults to PostgreSQL, API Gateway, web, and pipeline ports that can be occupied by another process. Git worktrees also derive their port sets from a path checksum, so two worktrees can map to the same set. Starting the stack then fails even though another local port set is available.

## Decision

`infra/host/common.sh` treats the four local service ports as one allocation unit. A normal checkout keeps the standard ports when they are available; a worktree starts from its checksum-derived set. When a generated set is stale or owned by another process, the host scripts search checksum-derived sets in a stable order and write the first fully available set to `infra/host/.env`. Listeners owned by the same checkout retain their ports. Explicit `CHORUZ_*_PORT` values and `CHORUZ_ENV=production` disable automatic allocation and are never changed automatically.

## Alternatives considered

**Choose each service port independently.** Rejected because the four URLs form one local stack contract; selecting a whole set keeps the mapping understandable and avoids partial reconfiguration.

**Fail on every occupied generated port.** Rejected because a worktree has no reason to require its first checksum candidate when a later stable candidate is available.

**Rewrite explicit environment overrides.** Rejected because an operator who supplies a port needs a deterministic failure for an invalid deployment configuration.

## Consequences

Normal checkouts and worktrees whose generated ports are occupied recover without manual port selection. Production and explicit deployment configuration stay deterministic. The allocation has the usual check-then-bind race, so a process that binds a selected port after the check still produces the service's ordinary bind error. The host lifecycle test pins foreign-listener avoidance, worktree-owned-listener reuse, and the production/explicit-override guardrails.

## Testing

`infra/host/tests/process_lifecycle.test.sh` verifies that a foreign listener advances the port set and that a listener owned by the current worktree does not force a new allocation.
