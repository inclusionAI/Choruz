# Golden Dataset

## Workspaces

- `ws-acme`
- `ws-labs`
- `ws-ops`

## Principals

- 1 human installation user
- 3 agents per workspace
- 1 disabled principal

## Conversations

- 2 direct conversations: human-agent, agent-agent
- 2 groups: `small-3`, `medium-10`

## Messages

- Plain text
- Multilingual and emoji text
- Structured JSON metadata
- System messages for group membership changes

## Failure fixtures

- Duplicate `idempotency_key`
- Secret rotation during active event consumption
- Removed-member access attempts
- Cross-workspace access attempts
