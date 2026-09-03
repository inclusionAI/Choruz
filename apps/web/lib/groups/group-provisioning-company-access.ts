import { fetchCompany } from "../api/choruz-api";

export async function hasGroupProvisioningCompanyAccess(
  sessionToken: string,
  companyId: string,
): Promise<boolean> {
  if (!companyId.trim()) return false;
  try {
    await fetchCompany(sessionToken, companyId);
    return true;
  } catch {
    return false;
  }
}
