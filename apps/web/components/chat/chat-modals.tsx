"use client";

import type { RuntimeHost } from "../../lib/remote/remote-control";
import type { Principal, Conversation, RuntimeBindingInfo, SearchResultItem, Company } from "../../lib/api/choruz-types";
import { CreateAgentModal } from "../agents/create-agent-modal";
import { CreateCompanyModal } from "../groups/create-company-modal";
import { CreateGroupModal } from "../groups/create-group-modal";
import { DetailPanel } from "./detail-panel";
import { HarnessAccountsModal } from "../agents/harness-accounts-modal";
import { ResizeHandle } from "../ui/resize-handle";
import type { PanelResize } from "../../hooks/use-panel-resize";
import { PixelWorldOverlay } from "../../plugins/pixel-world/client";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type ChatModalsProps = {
  // Pixel World
  showPixelWorld: boolean;
  pixelConversations: Conversation[];
  pixelAgents: Principal[];
  messagesByConv: Record<string, import("../../lib/api/choruz-types").ChatMessage[]>;
  activeConvId: string | null;
  onSelectConversation: (convId: string) => void;
  onClosePixelWorld: () => void;
  // Detail panel
  showDetail: boolean;
  activeConv: Conversation | null;
  principal: Principal;
  agents: Principal[];
  sessionToken: string;
  runtimeBindings: RuntimeBindingInfo[];
  runtimeHosts: RuntimeHost[];
  workspaceGitEnabled: boolean;
  agentSkillsEnabled: boolean;
  mathcodeEnabled: boolean;
  multiHarnessAccounts: boolean;
  onMultiHarnessAccountsChange: (enabled: boolean) => Promise<void>;
  onCloseDetail: () => void;
  refreshSnapshot: () => Promise<void>;
  detailResize: PanelResize;
  // Search
  searchQuery: string;
  searchResults: SearchResultItem[];
  searchLoading: boolean;
  onSearchInput: (value: string) => void;
  onSearchResultClick: (conversationId: string) => void;
  // Create group modal
  showCreateGroup: boolean;
  activeCompanyId: string | null;
  onCloseCreateGroup: () => void;
  onCreatedGroup: (convId: string) => void;
  // Create agent modal
  showCreateAgent: boolean;
  onCloseCreateAgent: () => void;
  onCreatedAgent: (convId: string) => void;
  showHarnessAccounts: boolean;
  onCloseHarnessAccounts: () => void;
  // Create company modal
  showCreateCompany: boolean;
  onCloseCreateCompany: () => void;
  onCreatedCompany: (company: Company) => void;
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function ChatModals({
  showPixelWorld,
  pixelConversations,
  pixelAgents,
  messagesByConv,
  activeConvId,
  onSelectConversation,
  onClosePixelWorld,
  showDetail,
  activeConv,
  principal,
  agents,
  sessionToken,
  runtimeBindings,
  runtimeHosts,
  workspaceGitEnabled,
  agentSkillsEnabled,
  mathcodeEnabled,
  multiHarnessAccounts,
  onMultiHarnessAccountsChange,
  onCloseDetail,
  refreshSnapshot,
  detailResize,
  searchQuery,
  searchResults,
  searchLoading,
  onSearchInput,
  onSearchResultClick,
  showCreateGroup,
  activeCompanyId,
  onCloseCreateGroup,
  onCreatedGroup,
  showCreateAgent,
  onCloseCreateAgent,
  onCreatedAgent,
  showHarnessAccounts,
  onCloseHarnessAccounts,
  showCreateCompany,
  onCloseCreateCompany,
  onCreatedCompany,
}: ChatModalsProps) {
  return (
    <>
      {/* ---- Pixel World panel ---- */}
      {showPixelWorld && (
        <PixelWorldOverlay
          conversations={pixelConversations}
          agents={pixelAgents}
          messagesByConv={messagesByConv}
          activeConvId={activeConvId}
          onSelectConversation={onSelectConversation}
          onClose={onClosePixelWorld}
        />
      )}

      {/* ---- Detail resize handle + panel ---- */}
      {showDetail && activeConv && <ResizeHandle resize={detailResize} label="Resize detail panel" />}
      {showDetail && activeConv && (
        <DetailPanel
          activeConv={activeConv}
          principal={principal}
          agents={agents}
          sessionToken={sessionToken}
          runtimeBindings={runtimeBindings}
          runtimeHosts={runtimeHosts}
          workspaceGitEnabled={workspaceGitEnabled}
          agentSkillsEnabled={agentSkillsEnabled}
          onClose={onCloseDetail}
          refreshSnapshot={refreshSnapshot}
          searchQuery={searchQuery}
          searchResults={searchResults}
          searchLoading={searchLoading}
          onSearchInput={onSearchInput}
          onSearchResultClick={onSearchResultClick}
          width={detailResize.width}
        />
      )}

      {/* ---- Create group modal ---- */}
      {showCreateGroup && (
        <CreateGroupModal
          principalId={principal.id}
          sessionToken={sessionToken}
          agents={agents}
          runtimeBindings={runtimeBindings}
          activeCompanyId={activeCompanyId}
          onClose={onCloseCreateGroup}
          onCreated={onCreatedGroup}
          refreshSnapshot={refreshSnapshot}
          agentSkillsEnabled={agentSkillsEnabled}
          multiHarnessAccounts={multiHarnessAccounts}
        />
      )}

      {/* ---- Create agent modal ---- */}
      {showCreateAgent && (
        <CreateAgentModal
          activeCompanyId={activeCompanyId}
          sessionToken={sessionToken}
          onClose={onCloseCreateAgent}
          onCreated={onCreatedAgent}
          refreshSnapshot={refreshSnapshot}
          agentSkillsEnabled={agentSkillsEnabled}
          mathcodeEnabled={mathcodeEnabled}
          multiHarnessAccounts={multiHarnessAccounts}
        />
      )}

      {showHarnessAccounts && activeCompanyId && (
        <HarnessAccountsModal
          companyId={activeCompanyId}
          sessionToken={sessionToken}
          multiHarnessAccounts={multiHarnessAccounts}
          onMultiHarnessAccountsChange={onMultiHarnessAccountsChange}
          onClose={onCloseHarnessAccounts}
        />
      )}

      {/* ---- Create company modal ---- */}
      {showCreateCompany && (
        <CreateCompanyModal
          principalId={principal.id}
          sessionToken={sessionToken}
          onClose={onCloseCreateCompany}
          onCreated={onCreatedCompany}
        />
      )}
    </>
  );
}
