import { lazy, Suspense, type ComponentProps } from "react";

import type { ClientPlugin } from "../client-plugin";

const PixelWorld = lazy(() => import("../../components/pixel-world/pixel-world"));

export const pixelWorldPlugin = {
  id: "pixel-world",
  version: "1",
  requiredHostCapabilities: ["workspace-roster", "conversation-activity"],
  clientCapabilities: ["sidebar-action", "workspace-overlay"],
} as const satisfies ClientPlugin;

export const pixelWorldSidebarAction = {
  id: "pixel-world",
  label: "Pixel World",
} as const;

export function PixelWorldOverlay(props: ComponentProps<typeof PixelWorld>) {
  return (
    <Suspense fallback={null}>
      <PixelWorld {...props} />
    </Suspense>
  );
}
