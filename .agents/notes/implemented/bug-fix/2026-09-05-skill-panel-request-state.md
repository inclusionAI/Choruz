# Agent Note: Skill panel request state

Status: implemented

## Problem

A failed skill-list request is indistinguishable from an empty installation, deletion failures have no visible recovery, and an older content response can overwrite the selected skill.

## Decision

`AgentSkillsList` keeps load and deletion failures visible without removing installed rows. Retry uses the existing list loader. An effect owns the selected skill's content request; cleanup prevents a response for a closed or replaced selection from changing the displayed content or loading state.

## Alternatives considered

**Treat errors as empty data.** This hides failures and incorrectly suggests that installed files are absent.

**Add a shared request manager.** These requests have one component owner; effect cleanup expresses their lifetime without a second cache or state layer.

## Consequences

The browser tests exercise the authenticated skill routes and owned files, replacing only a failed response or delaying a real content response. They verify retry, deletion recovery and replacement ordering without starting a harness. This does not change remote skill transport or filesystem authorization.
