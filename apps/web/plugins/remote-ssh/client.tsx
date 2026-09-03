import { lazy, Suspense, type ComponentProps } from "react";

import type { ClientPlugin } from "../client-plugin";

const ServerManager = lazy(() =>
  import("../../components/runtime/server-manager").then((module) => ({ default: module.ServerManager })),
);

export const remoteSshPlugin = {
  id: "remote-ssh",
  version: "1",
  requiredHostCapabilities: ["ssh-host-discovery", "ssh-tunnel-api", "remote-choruz-connect"],
  clientCapabilities: ["sidebar-action", "modal"],
} as const satisfies ClientPlugin;

export const remoteSshSidebarAction = {
  id: "remote-ssh",
  label: "Servers",
} as const;

export function RemoteSshModal(props: ComponentProps<typeof ServerManager>) {
  return (
    <Suspense fallback={null}>
      <ServerManager {...props} />
    </Suspense>
  );
}
