import type { HostPluginManifest } from "../lib/api/choruz-types";
import { hostSupportsClientPlugin, type ClientPlugin } from "./client-plugin";
import { kanbanPlugin } from "./kanban/client";
import { pixelWorldPlugin } from "./pixel-world/client";
import { workspaceGitPlugin } from "./workspace-git/client";
import { remoteSshPlugin } from "./remote-ssh/client";
import { remoteControlPlugin } from "./remote-control/client";
import { agentSkillsPlugin } from "./agent-skills/client";
import { mathcodePlugin } from "./mathcode/client";

const CLIENT_PLUGINS = [
  kanbanPlugin,
  pixelWorldPlugin,
  workspaceGitPlugin,
  remoteSshPlugin,
  remoteControlPlugin,
  agentSkillsPlugin,
  mathcodePlugin,
] satisfies readonly ClientPlugin[];

export function resolveClientPluginIds(hostPlugins: readonly HostPluginManifest[]): Set<string> {
  const hostById = new Map(hostPlugins.map((plugin) => [plugin.id, plugin]));
  return new Set(
    CLIENT_PLUGINS
      .filter((plugin) => hostSupportsClientPlugin(hostById.get(plugin.id), plugin))
      .map((plugin) => plugin.id),
  );
}
