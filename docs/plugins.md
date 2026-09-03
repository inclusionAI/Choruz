# Choruz Host/Client Plugins

Choruz ships product features on top of a small communication/runtime core as built-in Host/Client plugins. The Host independently registers each enabled plugin's routes and publishes its manifest. The Client activates a plugin's UI only when that manifest is compatible with its matching implementation. This prevents a Client from rendering controls for unavailable Host behavior and lets an older Client safely ignore newer Host plugins.

## Configuration

`CHORUZ_PLUGINS` is a comma-separated allowlist of built-in plugin IDs. When it is unset, all built-ins are enabled. Set it to an empty string to start the core product without plugins.

```bash
CHORUZ_PLUGINS=kanban,pixel-world,workspace-git,remote-ssh,agent-skills,mathcode pnpm dev:all
CHORUZ_PLUGINS=workspace-git,agent-skills pnpm dev:all
CHORUZ_PLUGINS= pnpm dev:all
```

The current built-ins are:

| Plugin | Host contribution | Client contribution |
|---|---|---|
| `kanban` | Channel-task HTTP routes and task command/event support | Conversation Tasks tab and create-from-message action |
| `pixel-world` | Workspace-roster and conversation-activity data contract | Sidebar action, overlay, and activity animations |
| `workspace-git` | Advertises authorized workspace-repository access | Git detail tab and guarded local Git graph API |
| `remote-ssh` | SSH host discovery, tunnel, and remote Choruz connection routes | Servers sidebar action and connection modal |
| `agent-skills` | Advertises authorized agent-workspace access | Skills detail tab, local skill management, and provisioning controls |
| `mathcode` | Advertises the MathCode terminal-agent driver | Enables MathCode in Create Agent; the existing availability guard requires a local `mathcode` CLI |

`mathcode` creates a new MathCode agent with the installed [`mathcode`](https://github.com/math-ai-org/mathcode) CLI. Install MathCode with its own setup procedure before selecting it. The plugin does not scan or import MathCode's historical sessions because MathCode does not publish a stable session-catalog contract for that purpose.

## Contract And Registration

The Host owns the authoritative plugin allowlist in `crates/choruz-common/src/plugins.rs`. `services/choruz-api-gateway/src/plugins/` supplies installable registration descriptors containing a manifest and optional router. The Core folds enabled descriptors into its router and publishes their manifests in the `/v1/console` `plugins` array; it does not need a feature-specific route switch. A disabled route-providing plugin does not register its routes.

The Client registry lives in `apps/web/plugins/registry.ts`. Each Client plugin declares its version and required Host/Client capabilities. The registry activates it only when the Host manifest has the same ID and version and includes every required capability. Unknown or incompatible Host plugins are ignored.

Plugin-specific Client entry points live under `apps/web/plugins/<plugin>/`. The shell consumes plugin entry points through stable contribution slots: conversation tabs, sidebar actions/modals, workspace detail tabs, agent detail tabs, and agent-provisioning controls. Web-local plugin APIs also check the same allowlist and return `404` while their plugin is disabled.

## Adding A Built-In Plugin

1. Add its ID to the common built-in catalog.
2. Add one Host registration descriptor (manifest plus an optional router) under `services/choruz-api-gateway/src/plugins/`.
3. Add one Client entry point and its contribution components under `apps/web/plugins/<id>/`.
4. Install the descriptor in the Host registration list and the Client entry point in `apps/web/plugins/registry.ts`.
5. If the plugin owns Next.js API routes, guard them with `serverPluginEnabled`.
6. Test Host registration, contract compatibility, disabled API behavior, and the plugin's real UI flow.

This is a build-time installation system: another plugin can be mounted on Choruz Core without adding feature-specific branches to the Core router, but its trusted code is still compiled with the product. Choruz does not execute downloaded third-party code at runtime; a marketplace would require signing, permissions, dependency isolation, migrations, and upgrade/rollback design first.
