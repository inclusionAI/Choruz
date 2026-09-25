export function serverPluginEnabled(
  pluginId: string,
  configuredPlugins = process.env.CHORUZ_PLUGINS,
): boolean {
  if (configuredPlugins === undefined) return pluginId !== "pi" && pluginId !== "opencode";
  return configuredPlugins
    .split(",")
    .map((item) => item.trim())
    .includes(pluginId);
}
