import { apiBaseUrl } from "../api/choruz-api";
import { transportFetch } from "../../lib/api/transport";

export async function canAccessHarnessAccountCompany(token: string, companyId: string): Promise<boolean> {
  const response = await transportFetch(`${apiBaseUrl()}/v1/companies`, {
    headers: { authorization: `Bearer ${token}` },
    cache: "no-store",
  });
  if (!response.ok) return false;
  const companies = await response.json() as Array<{ id?: string }>;
  return companies.some((company) => company.id === companyId);
}
