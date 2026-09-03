import { cookies } from "next/headers";
import { redirect } from "next/navigation";

// Prevent Next.js from automatically re-executing this server component.
// Without this, the dynamic page (due to cookies() call) gets revalidated
// periodically, which unmounts and remounts ChatApp, destroying all client
// state (Pixel World, WebSocket connections, scroll position, etc.).
export const revalidate = false;

import { ChatApp } from "../../components/chat/chat-app";
import {
  ApiRequestError,
  fetchDashboardBootstrap,
  sessionCookieName,
} from "../../lib/api/choruz-api";
import {
  EMPTY_DASHBOARD_PROPS,
  dashboardSnapshotFromBootstrap,
} from "../../lib/api/dashboard-snapshot";

export default async function DashboardPage() {
  const cookieStore = await cookies();
  const sessionToken = cookieStore.get(sessionCookieName())?.value;
  if (!sessionToken) {
    redirect("/");
  }

  // A failed bootstrap still renders the shell; the failure stays in the
  // server log so an outage is not mistaken for an empty account.
  let props = EMPTY_DASHBOARD_PROPS;
  try {
    props = dashboardSnapshotFromBootstrap(
      await fetchDashboardBootstrap(sessionToken, { limit: 100 }),
    );
  } catch (err) {
    if (err instanceof ApiRequestError && err.status === 401) {
      redirect("/auth/session-invalid");
    }
    console.error(
      `[dashboard] fetch failed source=dashboard_bootstrap error=${
        err instanceof Error ? err.message : String(err)
      }`,
    );
  }

  // NOTE: don't pass gatewayBaseUrl — server-side apiBaseUrl() is "http://127.0.0.1:3000"
  // which doesn't work from LAN/mobile browsers. Client computes the correct host
  // from window.location.hostname at runtime.
  return <ChatApp sessionToken={sessionToken} {...props} />;
}
