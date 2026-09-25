# Agent Note: Optional Pi and OpenCode harness plugins

Status: implemented

## Problem

Pi and OpenCode appear as core creation and import choices even when operators do not want those integrations. Removing their runtime implementations would strand existing Agents and recorded sessions.

## Decision

Pi and OpenCode use the same built-in Host/Client plugin registration mechanism as [MathCode](2026-09-03-mathcode-agent-plugin.md), with IDs `pi` and `opencode`. Neither is enabled by default. The controller's allowlist controls compatible client choices, the provisioning route, and the session scan/import routes. The execution device still owns its CLI installation and availability checks. MathCode keeps its existing default.

Driver identity and execution remain independent of new-agent eligibility. Existing bindings and historical data are not migrated or deleted when a plugin is disabled. Account management remains limited to Claude Code and Codex.

## Alternatives considered

**Delete the drivers.** This would prevent existing Agents from resuming and discard useful opt-in functionality.

**Hide only the dropdown.** The import flow and direct provisioning requests would still expose the supposedly optional integrations.

**Introduce downloadable plugin packages.** The existing MathCode mechanism is a compiled-in integration with operator activation, not a package marketplace. A separate installation framework is unnecessary for this change.

## Consequences

Operators must install the CLI separately and explicitly include the plugin ID in the web/API allowlist to create or import Agents. The same driver-plugin mapping replaces the MathCode-specific creation switch. Runtime and transcript parsers stay available for persisted Agents; plugin disablement is not a kill switch.
