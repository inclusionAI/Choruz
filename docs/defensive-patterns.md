# Defensive patterns

Hard-won bug-class rules: each pattern below is a class of defect that actually shipped or nearly shipped here, stated as the rule that prevents its recurrence. Read this before writing message-path, lifecycle, migration, test-fixture or bulk-edit code. A new entry needs a real incident behind it and the Agent Note or pull request that fixed it.

## Reconcile optimistic state by client key, never by server id alone

The web client inserts an optimistic message whose id is its idempotency key, and the server copy can arrive by more than one path: the sync feed's `message.created`, a bootstrap refresh that merges a conversation's `last_message` preview, or a page fetch. Deduplicating only by server id let a preview cache the server copy first, after which the sync event returned early and the optimistic bubble stayed forever: two bubbles per message. Every merge path into the message cache goes through one helper (`upsertConfirmedMessage` in `apps/web/lib/messages/messages.ts`) that replaces or removes the optimistic entry whose `idempotency_key` matches, whichever copy arrives second.

## Applied migrations are frozen; a bulk edit must exclude them

`scripts/historical-migrations.sha256` pins every applied migration and the DB smoke job verifies it. A repository-wide path rewrite (`sed` over `git grep -l`) reached a comment inside `V018__message_threads.sql` and turned a documentation move into a red smoke job, twice in one day. Any bulk edit (`sed -i`, a formatter, or a find-and-replace) excludes `migrations/` and `services/choruz-pipeline/src/instructions_fixtures/`; a schema or data change is always a new `V0NN__name.sql`.

## A test owns the data it asserts on

The e2e suite runs with two fully parallel workers per shard against one PostgreSQL and one API. A test that asserted "at least one agent exists", compared sidebar counts taken 300 ms apart, or clicked the first conversation item passed alone and failed beside its neighbours. Every test seeds its own group, agent or message with `uniqueName(...)` and asserts on those ids; it never assumes an empty workspace, never compares absolute counts, and never picks "the first" of a shared list. [choruz-ci-test-reliability](../.agents/skills/choruz-ci-test-reliability/SKILL.md) owns the full rule set.

## Readiness is a signal, not a sleep

The performance smoke backgrounded `cargo run` and polled the gateway for a fixed window; on a cold cache the build alone exceeded it and the job failed with a misleading "gateway never became ready". Build first in the foreground, then start, then wait on the readiness endpoint. In tests, wait on an observable condition (`expect.poll`, `toHaveCount`, a `/readyz` response), never on a duration.

## Cut a file by its structure, never by line numbers

Removing a block from a shell script by a line range cut through the middle of a `node` heredoc and left a script that no longer parsed. Delete by the construct's own delimiters (the heredoc terminator, the function's closing brace, the matching fence) and run `bash -n` (or the language's parser) before committing.

## Route children's stdout away from a stdout handshake

`choruz-server` prints one `CHORUZ_LISTENING=<port>` line on stdout that the SSH client parses. A child process inheriting stdout would interleave its tracing into that channel. Children spawned by the supervisor get stdout redirected to the parent's stderr (`dup_stderr_to_stdio` in `crates/choruz-supervisor/src/supervisor.rs`); any new handshake on stdout keeps that rule.

## Never edit a frozen artefact to fix a reference

When a document moves, the citations inside frozen artefacts (applied migrations, archived notes, compatibility fixtures) stay as they are; the current document carries the forwarding context. Rewriting the frozen file is the bug.
