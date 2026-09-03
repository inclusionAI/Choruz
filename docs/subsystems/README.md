# Subsystems

One page per subsystem: what it is, what it owns, the data structures and messages it moves, its entry points, and the invariants it keeps. The folder complements [architecture.md](../architecture.md), which describes behaviour across subsystems; a subsystem page never narrates the whole flow, it states its own contract and links its neighbours.

| Page | Owns |
|---|---|
| [choruz-api-gateway](api-gateway.md) | HTTP and WebSocket surface, authentication, request validation, the `/v1` routes |
| [message-pipeline](message-pipeline.md) | CDC intake, router, executor, writer, fanout, cron and the outbox watcher in `services/choruz-pipeline` |
| [agent-protocol](agent-protocol.md) | The `[choruz-incoming]` envelope, `$CHORUZ_SEND` commands, the maildir outbox, instruction bootstrap and refresh |
| [choruz-agent-runtime](agent-runtime.md) | Terminal drivers (Claude Code, Codex, Gemini), sessions, bindings, harness accounts, `crates/choruz-agent-runtime` and `crates/choruz-session` |
| [store](store.md) | `crates/choruz-application` `DbService`, `crates/choruz-store`, workspaces, conversations, messages, `server_seq`, idempotency |
| [sync-feed](sync-feed.md) | The change log, devices, `sync_change`, bootstrap pages, unread counters, how the web client stays current |
| [web-client](web-client.md) | `apps/web`: chat app, message cache and IndexedDB, hooks, docs site, e2e harness |
| [channel-tasks](channel-tasks.md) | The kanban board: tasks, assignees, roster, receipts, provisioning jobs |
| [threads](threads.md) | Message threads: roots, replies, broadcast, thread unread state |
| [host-and-remote](host-and-remote.md) | `choruz-supervisor`, `choruz-server`, connector, remote-control gateway, SSH tunnels, runtime hosts |
| [bridge](bridge.md) | `services/choruz-bridge`: Slack and Telegram adapters, mapping store, webhook server |

## Page contract

Every page follows the same skeleton so a reader can find the same thing in the same place:

```markdown
# <Subsystem name>

<One paragraph: what it is and what a reader can do with it. Source: the owning crate or directory, linked.>

## Owns
## Data
## Entry points
## Invariants
## Failure modes
## Tests
## Related
```

- **Owns**: the crates, directories, tables and endpoints this subsystem is responsible for, by path.
- **Data**: the types, rows and messages it moves, with the real field names; link the source file rather than restating a whole struct.
- **Entry points**: how work enters (HTTP route, event type, CLI command, cron) and where it leaves.
- **Invariants**: what is always true, and which test or check pins each one.
- **Failure modes**: what happens on the known failures and how an operator sees them.
- **Tests**: where the unit, integration and e2e coverage lives.
- **Related**: neighbouring subsystem pages and the Agent Notes that own the decisions.

Rules: present tense, current state only, every claim read from the current checkout (a path that does not exist is a defect in the page), one physical line per paragraph, relative links. A change that moves a file, renames a type or adds an endpoint updates the page in the same pull request.
