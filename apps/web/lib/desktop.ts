import { activeTransport, localTransport } from "./api/transport";

declare global {
  interface Window {
    choruzDesktop?: { chooseFolder(): Promise<string | null> };
  }
}

/** Native dialogs select this computer's paths, never a remote device's paths. */
export function nativeFolderPicker(runtimeHostId?: string | null) {
  if (typeof window === "undefined" || runtimeHostId || activeTransport() !== localTransport) return undefined;
  return window.choruzDesktop?.chooseFolder;
}
