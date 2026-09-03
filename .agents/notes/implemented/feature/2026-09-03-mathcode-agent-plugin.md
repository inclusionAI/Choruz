# Agent Note: MathCode agent plugin

Status: implemented

## Problem

MathCode provides a local terminal agent for Lean formalization and proof work, but Choruz could not provision it as a named agent or express its availability through the Host/Client plugin contract.

## Decision

The built-in `mathcode` plugin exposes a `mathcode_terminal` driver. A compatible Host manifest enables the Create Agent selector, and the provisioning route rejects the driver while the plugin is disabled. The driver resolves `CHORUZ_MATHCODE_BINARY` before `mathcode`, starts an interactive terminal without synthetic CLI flags, and uses MathCode's documented `-p` mode for headless prompts. MathCode selects its model through its own configuration, so Choruz does not forward a model override.

MathCode historical sessions are not included in the workspace-session importer because the upstream project does not publish a stable session-catalog format.

## Alternatives considered

**Treat MathCode as Codex.** MathCode can use Codex OAuth, but it has its own executable, Lean workspace behavior, and prompt protocol; recording it as `codex_terminal` would launch the wrong binary and hide the dependency from operators.

**Add it as an always-on core driver.** The product plugin contract exists to let hosts advertise optional capabilities. Keeping this driver behind `mathcode` preserves an explicit operator choice and prevents clients from offering an unavailable integration.

**Guess a session importer from local files.** A guessed private storage format would turn imported agents into fragile resume targets, so the plugin creates new agents only.

## Consequences

Operators install MathCode separately, enable `mathcode` in `CHORUZ_PLUGINS`, and can create a MathCode agent from the normal Create Agent flow. An unavailable executable blocks creation through the existing driver-availability check. Historical MathCode work remains outside the import flow until upstream offers a stable catalog contract.

## Testing

The driver tests pin prompt arguments and plain-output handling; plugin registry, provisioning-route, driver-availability, and terminal tests pin the manifest and disabled-plugin behavior.
