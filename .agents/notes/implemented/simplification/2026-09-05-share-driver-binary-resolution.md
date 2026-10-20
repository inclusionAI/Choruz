# Agent Note: Share driver binary resolution

Status: implemented

## Problem

Availability and model discovery need the same executable metadata and override precedence. Duplicating those definitions lets the two callers advertise different local executables.

## Decision

The existing TypeScript driver registry owns binary definitions and resolution for availability and model discovery: trimmed `CHORUZ_*_BINARY`, then the supported trimmed `CHORUZ_*_CLI_PATH`, then the driver default. MathCode retains its binary-only override. Provisioning does not resolve or persist an automatic executable; the executing device owns that selection, as described in [device operations](../architecture/2026-09-04-device-operations-behind-runtime-host.md).

The registry also owns driver display labels and availability setup hints. Availability iterates those definitions and supplies only executable and runtime probe results; UI labels use the same registry lookup. Creation capabilities, instruction files and Rust execution retain their respective owners. The resolver uses its supplied environment and performs no device discovery.

## Alternatives considered

**Keep the provisioning map.** It resolves the controller's environment rather than the executing device's, so sharing its implementation would not fix its ownership.

**Unify all driver execution across languages and devices.** Driver capabilities and remote execution have distinct owners. A shared TypeScript resolver cannot decide which device environment is authoritative.

## Consequences

Local availability and model discovery agree on overrides, and availability labels agree with the UI. Creating an Agent leaves automatic executable selection to its device; explicit per-agent paths retain target-device meaning. Caller tests cover shared discovery precedence, canonical labels and the absence of controller executable persistence, while the runtime owner's connector regression verifies actual launch selection.
