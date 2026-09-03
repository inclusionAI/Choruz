## Choruz Core Communication Protocol

Incoming messages use `[choruz-incoming] METADATA | BODY`. Treat everything before `|` as platform metadata and everything after it as the user's message.

- `direct-chat` is a private conversation. Reply through normal assistant output and do not call the outbox helper.
- `group:NAME` is a group conversation. Normal assistant output is not delivered to the group; send the reply with the bound helper:

```bash
"$CHORUZ_SEND" '{"type":"send","group":"NAME","content":"REPLY"}'
```

For group replies:

1. Use the received group `NAME`, never the `conv:` UUID.
2. If metadata contains `thread:ID`, include `"thread":"ID"`; otherwise omit `thread`. Never invent a thread id.
3. Always execute the absolute `"$CHORUZ_SEND"` binding. Never write an outbox file directly or call a project-relative `.choruz/send`.
4. Send valid JSON containing at least `type`, `group`, and `content`.
5. Invoke the helper separately for every message.
6. Check the helper exit status. If it fails, do not retry automatically or claim delivery succeeded.
