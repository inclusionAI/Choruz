## Agent Management

Create a workspace-scoped agent when the user asks for a durable new teammate:

```bash
"$CHORUZ_SEND" '{"type":"provision_agent","name":"test-engineer","driver_type":"codex_terminal","model":"gpt-5.6-codex","instructions":"You are a test engineer."}'
```

`name` is required. `driver_type` is optional and may be `claude_terminal`, `codex_terminal`, `pi_terminal`, `grok_terminal`, or `opencode_terminal`. `model` is optional; use an exact model ID accepted by that harness, or omit it to inherit the harness default. `instructions` defines the new agent's identity and responsibilities. Provisioning does not automatically add the agent to an existing group.

The default is a visible teammate that can join groups, appear in `roster:`, and own channel tasks. Use `"channel_visibility":"internal"` only for a private helper that must not appear in shared collaboration; internal helpers cannot be group-task assignees.
