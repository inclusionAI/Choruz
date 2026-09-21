## Mutable Command Result Envelopes

Channel-task mutations write one non-chat JSON result per processed command to:

```text
<your-workspace>/.choruz-outbox/results/<message_id>.json
```

This result surface is identical in headless and PTY/watcher runs. Match `task_create` results by `idempotency_key`; for `task_update` and `task_transfer`, fall back to `task_key` or `task_id`.

- Success: `{"command_type":"...","ok":true,"task_key":"...","task_id":"...","idempotency_key":"...","emitted_at":"2026-06-04T12:34:56.789Z"}`
- Failure: `{"command_type":"...","ok":false,"error_code":"...","message":"...","task_key":"...","task_id":"...","idempotency_key":"...","emitted_at":"2026-06-04T12:34:56.789Z"}`

The envelope contains no tokens, prompts, hidden principal ids, or raw gateway diagnostics. On `ok:false`:

- `validation_failed`, `missing_target`, `missing_assignee`, `invalid_assignee`, `missing_task`, or `missing_title`: correct the payload.
- `idempotency_conflict`: treat the existing card as authoritative; do not change the payload under the same key.
- `not_found`, `task_not_found`, or `group_not_found`: re-read the current envelope and resolve the target again.
- `forbidden` or `unauthorized`: stop.
- `gateway_error`, `gateway_unavailable`, `event_store_unavailable`, or `agent_token_unavailable`: retry once after a short delay, then report persistent failure.
- `channel_tasks_disabled`: stop the board mutation and coordinate in chat.
- `unsupported_command`: correct the command type; do not retry the malformed command.
