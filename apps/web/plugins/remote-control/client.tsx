import { lazy, Suspense, type ComponentProps } from "react";

import type { ClientPlugin } from "../client-plugin";

const RemoteControlManager = lazy(() =>
  import("../../components/runtime/remote-control-manager").then((module) => ({
    default: module.RemoteControlManager,
  })),
);

const RuntimeHostManager = lazy(() =>
  import("../../components/runtime/runtime-host-manager").then((module) => ({
    default: module.RuntimeHostManager,
  })),
);

export const remoteControlPlugin = {
  id: "remote-control",
  version: "1",
  requiredHostCapabilities: [
    "opaque-pairing-credential",
    "remote-device-api",
    "cloud-gateway",
    "end-to-end-encryption",
  ],
  clientCapabilities: ["sidebar-action", "modal", "web-dashboard"],
} as const satisfies ClientPlugin;

export const remoteControlSidebarAction = {
  id: "remote-control",
  label: "Remote Control",
} as const;

export function RemoteControlModal(props: ComponentProps<typeof RemoteControlManager>) {
  return (
    <Suspense fallback={null}>
      <RemoteControlManager {...props} />
    </Suspense>
  );
}

export function RuntimeHostsModal(props: ComponentProps<typeof RuntimeHostManager>) {
  return (
    <Suspense fallback={null}>
      <RuntimeHostManager {...props} />
    </Suspense>
  );
}
