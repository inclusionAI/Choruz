# Agent Note: Muse core driver and optional Grok integration

Status: implemented

## Problem

Operators need Muse Code as a built-in execution choice without requiring Grok installation or removing the ability to use existing Grok Agents.

## Decision

Muse uses the shared driver identity, executable resolution, PTY, headless parser, instruction bootstrap and local/remote host dispatch. The official CLI owns credentials. Headless turns consume versioned Muse session records and persist the exact session identity. Native approvals stay enabled in the interactive terminal; unattended headless turns disable approval prompts but retain the sandbox.

Grok follows the [optional harness plugin contract](2026-09-25-optional-harness-plugins.md): the `grok` manifest gates creation and historical session import, not execution of existing bindings. Codex remains a core driver. The schema adds Muse without rewriting existing bindings.

## Alternatives considered

**Only relabel a driver.** Muse's flags and event schema differ from Codex; reusing its identity would corrupt execution and retained sessions.

**A separate Muse runtime.** Shared transport, process containment and outbox delivery already own these concerns. A second runtime would duplicate authorization and cleanup.

**Delete Grok execution.** Optional activation must not strand existing bindings or delete recorded data.

## Consequences

Muse requires its official CLI and login on each execution device. It supports terminal and group execution, but does not claim model discovery, isolated account management or history import. The dedicated session protocol is not presented as a Codex-compatible protocol. The Pi/OpenCode note remains active because its negative guarantees and operator activation model also constrain Grok.
