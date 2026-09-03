"use client";

import { useCallback, useState } from "react";
import { apiFetch } from "../lib/api/choruz-api";
import type { Company } from "../lib/api/choruz-types";

/**
 * The company list plus the mutations the sidebar's company menu offers.
 * Every mutation updates the list in place from the server's answer.
 * Which company is active is the caller's state: switching it also means
 * switching tabs and the open conversation.
 */
export function useCompanyManagement({
  initialCompanies,
  sessionToken,
  actorId,
}: {
  initialCompanies: Company[];
  sessionToken: string;
  actorId: string;
}) {
  const [companies, setCompanies] = useState<Company[]>(initialCompanies);

  const addCompany = useCallback((company: Company) => {
    setCompanies((prev) => [...prev, company]);
  }, []);

  const patchCompany = useCallback(
    (companyId: string, fields: Record<string, unknown>) =>
      apiFetch<Company>(`/v1/companies/${companyId}`, sessionToken, {
        method: "PATCH",
        body: JSON.stringify({ actor_id: actorId, ...fields }),
      }),
    [actorId, sessionToken],
  );

  const postCompanyAction = useCallback(
    (companyId: string, action: "archive" | "unarchive") =>
      apiFetch<Company>(`/v1/companies/${companyId}/${action}`, sessionToken, {
        method: "POST",
        body: JSON.stringify({ actor_id: actorId }),
      }),
    [actorId, sessionToken],
  );

  const replaceCompany = useCallback((companyId: string, updated: Company) => {
    setCompanies((prev) => prev.map((c) => (c.id === companyId ? updated : c)));
  }, []);

  const toggleAgentsActive = useCallback(
    async (companyId: string, active: boolean) => {
      await patchCompany(companyId, { agents_active: active });
      setCompanies((prev) => prev.map((c) => (c.id === companyId ? { ...c, agents_active: active } : c)));
    },
    [patchCompany],
  );

  const setMultiHarnessAccounts = useCallback(
    async (companyId: string, enabled: boolean) =>
      replaceCompany(companyId, await patchCompany(companyId, { multi_harness_accounts: enabled })),
    [patchCompany, replaceCompany],
  );

  const archiveCompany = useCallback(
    async (companyId: string) => replaceCompany(companyId, await postCompanyAction(companyId, "archive")),
    [postCompanyAction, replaceCompany],
  );

  const unarchiveCompany = useCallback(
    async (companyId: string) => replaceCompany(companyId, await postCompanyAction(companyId, "unarchive")),
    [postCompanyAction, replaceCompany],
  );

  const deleteCompany = useCallback(
    async (companyId: string) => {
      await apiFetch<void>(`/v1/companies/${companyId}`, sessionToken, { method: "DELETE" });
      setCompanies((prev) => prev.filter((c) => c.id !== companyId));
    },
    [sessionToken],
  );

  const renameCompany = useCallback(
    async (companyId: string, name: string) => {
      await patchCompany(companyId, { name });
      setCompanies((prev) => prev.map((c) => (c.id === companyId ? { ...c, name } : c)));
    },
    [patchCompany],
  );

  const changeCompanyWorkspace = useCallback(
    async (companyId: string, folderPath: string) =>
      replaceCompany(companyId, await patchCompany(companyId, { folder_path: folderPath })),
    [patchCompany, replaceCompany],
  );

  return {
    companies,
    replaceCompanies: setCompanies,
    addCompany,
    toggleAgentsActive,
    setMultiHarnessAccounts,
    archiveCompany,
    unarchiveCompany,
    deleteCompany,
    renameCompany,
    changeCompanyWorkspace,
  };
}
