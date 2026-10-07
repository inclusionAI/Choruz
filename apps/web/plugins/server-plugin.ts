export function serverPluginEnabled(
  pluginId: string,
  configuredPlugins = process.env.CHORUZ_PLUGINS,
): boolean {
  if (configuredPlugins === undefined) return !["pi", "opencode", "grok"].includes(pluginId);
  return configuredPlugins
    .split(",")
    .map((item) => item.trim())
    .includes(pluginId);
}
