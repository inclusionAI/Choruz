"use client";

import { useEffect, useState } from "react";
import { activeTransport, localTransport, transportFetch } from "../lib/api/transport";

/** Remote DOM resources need owned blob URLs; local and external URLs stay native.
 * Revokes the blob and discards late responses when its consumer changes or unmounts. */
export function useTransportUrl(path: string | undefined, load = true) {
  const remote = Boolean(path?.startsWith("/api/") && activeTransport() !== localTransport);
  const [resource, setResource] = useState<{ path: string; url?: string; error?: string }>();
  useEffect(() => {
    if (!remote || !path || !load) return;
    let cancelled = false;
    let url: string | undefined;
    setResource({ path });
    void transportFetch(path).then(async (response) => {
      if (!response.ok) throw new Error(`Attachment request failed (${response.status})`);
      const blob = await response.blob();
      if (cancelled) return;
      url = URL.createObjectURL(blob);
      setResource({ path, url });
    }).catch((error: unknown) => {
      if (!cancelled) setResource({ path, error: String(error) });
    });
    return () => {
      cancelled = true;
      if (url) URL.revokeObjectURL(url);
    };
  }, [path, remote, load]);
  return {
    remote,
    url: remote ? (resource?.path === path ? resource?.url : undefined) : path,
    error: resource?.path === path ? resource?.error : undefined,
  };
}
