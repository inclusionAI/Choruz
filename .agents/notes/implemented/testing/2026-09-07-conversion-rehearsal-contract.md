# Agent Note: Conversion rehearsal proves discard, not execution

Status: implemented

## Problem

Running one fixture under two policy names duplicates PostgreSQL setup without
proving two behaviours. Moving queued files into evidence cannot demonstrate
that a command executor drained them successfully.

## Decision

The offline conversion rehearsal runs one discard-and-restore journey. It checks
the archived command bytes against the backup and retains the stopped-writer,
collision, identifier and restore checks. Its private Unix socket avoids a
shared TCP port; unsuccessful PostgreSQL shutdown preserves the fixture.

## Alternatives considered

**Keep both labels.** Repeating identical operations adds cost and falsely
implies executor coverage.

**Build a synthetic drain executor here.** Another test-only implementation
would not prove production command execution. Recovery acceptance belongs to
the real pipeline, independently of this offline filesystem rehearsal.

## Consequences

The command accepts no policy argument. Drain remains outside its acceptance
claim. The miniature database checks backup restoration, not production schema
migrations or live-installation conversion.
