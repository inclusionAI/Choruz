import { cookies } from "next/headers";
import { redirect } from "next/navigation";

import { sessionCookieName } from "../lib/api/choruz-api";

function localPort(value: string | undefined, fallback: number): number {
  const port = Number(value ?? fallback);
  return Number.isInteger(port) && port > 0 && port <= 65_535 ? port : fallback;
}

export default async function LocalEntryPage() {
  const cookieStore = await cookies();
  if (cookieStore.get(sessionCookieName())?.value) {
    redirect("/dashboard");
  }

  const apiPort = localPort(process.env.CHORUZ_API_PORT, 3000);
  const webPort = localPort(process.env.CHORUZ_WEB_PORT, 3100);
  redirect(
    `http://127.0.0.1:${apiPort}/v1/auth/local/bootstrap?return_port=${webPort}`,
  );
}
