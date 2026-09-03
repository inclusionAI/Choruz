import type { HostPluginManifest } from "../lib/api/choruz-types";

export type ClientPlugin = {
  id: string;
  version: string;
  requiredHostCapabilities: readonly string[];
  clientCapabilities: readonly string[];
};

export function hostSupportsClientPlugin(
  hostPlugin: HostPluginManifest | undefined,
  clientPlugin: ClientPlugin,
): boolean {
  return Boolean(
    hostPlugin &&
      hostPlugin.version === clientPlugin.version &&
      clientPlugin.requiredHostCapabilities.every((capability) =>
        hostPlugin.host_capabilities.includes(capability),
      ) &&
      clientPlugin.clientCapabilities.every((capability) =>
        hostPlugin.client_capabilities.includes(capability),
      ),
  );
}
