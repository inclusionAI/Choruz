import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import { Dashboard } from "../../components/chat/dashboard";
import { sessionCookieName } from "../../lib/api/choruz-api";

export const revalidate = false;

export default async function DashboardPage() {
  const sessionToken = (await cookies()).get(sessionCookieName())?.value;
  if (!sessionToken) redirect("/");
  return <Dashboard sessionToken={sessionToken} />;
}
