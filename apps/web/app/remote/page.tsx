import type { Metadata } from "next";

import { RemoteDashboard } from "../../components/remote/remote-dashboard";

export const metadata: Metadata = {
  title: "Choruz Remote",
};

// Client-rendered on purpose: the page pairs with a host through the Cloud
// Gateway and then runs the dashboard over the relay transport, so it needs
// no session cookie and no server-side data.
export default function RemotePage() {
  return <RemoteDashboard />;
}
