"use client";

import { X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { trace } from "../../lib/api/choruz-trace";
import type { Principal, Conversation, RuntimeBindingInfo, SearchResultItem } from "../../lib/api/choruz-types";
import { AgentConfigEditor } from "../agents/agent-config-editor";
import { MemberRow, type MemberInfo } from "./member-row";
import { AgentSkillsPanel, agentSkillsDetailTab } from "../../plugins/agent-skills/client";
import { WorkspaceGitPanel, workspaceGitDetailTab } from "../../plugins/workspace-git/client";
import { RuntimeStatusPanel } from "../runtime/runtime-status-panel";
import { Avatar } from "../ui/avatar";
import { principalName, isAgent, conversationDisplayName } from "../../lib/api/principals";
import { agentPeerId, bindingMachineLabel } from "../../lib/terminal/terminal-bindings";
import type { RuntimeHost } from "../../lib/remote/remote-control";
import { apiFetch } from "../../lib/api/choruz-api";
import { EmptyState } from "../ui/empty-state";

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export type DetailPanelProps = {
  activeConv: Conversation;
  principal: Principal;
  agents: Principal[];
  sessionToken: string;
  runtimeBindings: RuntimeBindingInfo[];
  /** Paired machines of the active company, for naming where an agent runs. */
  runtimeHosts: RuntimeHost[];
  workspaceGitEnabled: boolean;
  agentSkillsEnabled: boolean;
  onClose: () => void;
  refreshSnapshot: () => Promise<void>;
  searchQuery: string;
  searchResults: SearchResultItem[];
  searchLoading: boolean;
  onSearchInput: (value: string) => void;
  onSearchResultClick: (conversationId: string) => void;
  width?: number;
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function DetailPanel({
  activeConv,
  principal,
  agents,
  sessionToken,
  runtimeBindings,
  runtimeHosts,
  workspaceGitEnabled,
  agentSkillsEnabled,
  onClose,
  refreshSnapshot,
  searchQuery,
  width,
  searchResults,
  searchLoading,
  onSearchInput,
  onSearchResultClick,
}: DetailPanelProps) {
  const chatTitle = conversationDisplayName(activeConv, principal, agents);
  const [showAddMember, setShowAddMember] = useState(false);
  const [addMemberSearch, setAddMemberSearch] = useState("");
  const [addingIds, setAddingIds] = useState<Set<string>>(new Set());
  const [addError, setAddError] = useState<string | null>(null);

  const activeMembers = useMemo(() => {
    return Object.keys(activeConv.members).map((id) => ({
      id,
      name: principalName(principal, agents, id),
      isAgent: isAgent(agents, id),
    }));
  }, [activeConv, principal, agents]);

  // Agents not yet in this conversation — scoped to the same workspace (company)
  const availableAgents = useMemo(() => {
    const memberIds = new Set(Object.keys(activeConv.members));
    return agents
      .filter((a) => !a.disabled && !memberIds.has(a.id) && a.workspace_id === activeConv.workspace_id)
      .sort((a, b) => a.name.localeCompare(b.name));
  }, [agents, activeConv.members, activeConv.workspace_id]);

  const filteredAvailable = useMemo(() => {
    if (!addMemberSearch.trim()) return availableAgents;
    const q = addMemberSearch.toLowerCase();
    return availableAgents.filter((a) => a.name.toLowerCase().includes(q));
  }, [availableAgents, addMemberSearch]);

  const handleRemoveMember = useCallback(async (member: MemberInfo) => {
    const span = trace.start("kick_member", { convId: activeConv.id, memberId: member.id });
    try {
      await apiFetch(
        `/v1/groups/${activeConv.id}/members/${member.id}?actor_id=${encodeURIComponent(principal.id)}`,
        sessionToken,
        { method: "DELETE" },
      );
      span.end({ status: "ok" });
      await refreshSnapshot();
    } catch (err) {
      span.end({ error: String(err) });
      window.alert(`Failed to remove ${member.name}: ${String(err)}`);
    }
  }, [activeConv.id, principal.id, sessionToken, refreshSnapshot]);

  const handleAddMember = useCallback(async (agentId: string) => {
    setAddingIds((prev) => new Set(prev).add(agentId));
    setAddError(null);
    const agentObj = agents.find((a) => a.id === agentId);
    const span = trace.start("add_member", { convId: activeConv.id, agentId, agentName: agentObj?.name });
    try {
      await apiFetch(
        `/v1/groups/${activeConv.id}/members`,
        sessionToken,
        {
          method: "POST",
          body: JSON.stringify({
            actor_id: principal.id,
            member_ids: [agentId],
          }),
        },
      );
      span.end({ status: "ok" });
      await refreshSnapshot();
      setAddingIds((prev) => {
        const next = new Set(prev);
        next.delete(agentId);
        return next;
      });
    } catch (err) {
      span.end({ error: String(err) });
      setAddError(String(err));
      setAddingIds((prev) => {
        const next = new Set(prev);
        next.delete(agentId);
        return next;
      });
    }
  }, [activeConv.id, principal.id, sessionToken, refreshSnapshot, agents]);

  // Find the agent binding — only for direct chats (1-on-1 with an agent).
  const agentBinding = useMemo(() => {
    if (activeConv.conversation_type !== "direct") return undefined;

    const convBindings = runtimeBindings.filter(
      (b) => b.conversation_id === activeConv.id,
    );
    if (convBindings.length > 0) return convBindings[0];

    const otherMemberId = agentPeerId(activeConv, principal.id, agents);
    if (otherMemberId) {
      const agentBindings = runtimeBindings.filter(
        (b) => b.agent_principal_id === otherMemberId,
      );
      return agentBindings.find((b) => b.conversation_type === "direct") ?? agentBindings[0];
    }

    return undefined;
  }, [activeConv, principal, agents, runtimeBindings]);

  // Tab state — DM defaults to "overview", group defaults to "members"
  const [activeTab, setActiveTab] = useState(() =>
    agentBinding ? "overview" : "members",
  );

  // Reset tab when conversation changes
  useEffect(() => {
    setActiveTab(agentBinding ? "overview" : "members");
    trace.event("detail_open", { convId: activeConv.id, convName: chatTitle });
  }, [activeConv.id, agentBinding, chatTitle]);

  useEffect(() => {
    if (activeTab === agentSkillsDetailTab.id && !agentSkillsEnabled) setActiveTab("overview");
    if (activeTab === workspaceGitDetailTab.id && !workspaceGitEnabled) setActiveTab("members");
  }, [activeTab, agentSkillsEnabled, workspaceGitEnabled]);

  return (
    <aside className="detail-panel" style={width ? { width, minWidth: width } : undefined}>
      <div className="detail-header">
        <h3>Details</h3>
        <button type="button" onClick={onClose} title="Close" aria-label="Close details">
          <X size={16} aria-hidden="true" />
        </button>
      </div>

      {/* Conversation info — always visible above tabs */}
      <div className="detail-section">
        <div className="detail-identity">
          <Avatar name={chatTitle} />
          <h3 className="detail-identity-title">{chatTitle}</h3>
          <p className="detail-identity-kind">
            {activeConv.conversation_type === "group" ? "Group" : "Direct"}{" "}
            conversation
          </p>
        </div>
      </div>

      {/* Tab bar */}
      <div className="detail-tabs">
        {agentBinding ? (
          <>
            <button
              className={`detail-tab${activeTab === "overview" ? " active" : ""}`}
              onClick={() => { setActiveTab("overview"); trace.event("detail_tab_switch", { tab: "overview" }); }}
            >
              Overview
            </button>
            <button
              className={`detail-tab${activeTab === "config" ? " active" : ""}`}
              onClick={() => { setActiveTab("config"); trace.event("detail_tab_switch", { tab: "config" }); }}
            >
              Config
            </button>
            {agentSkillsEnabled && (
              <button
                className={`detail-tab${activeTab === agentSkillsDetailTab.id ? " active" : ""}`}
                onClick={() => { setActiveTab(agentSkillsDetailTab.id); trace.event("detail_tab_switch", { tab: agentSkillsDetailTab.id }); }}
              >
                {agentSkillsDetailTab.label}
              </button>
            )}
          </>
        ) : (
          <>
            <button
              className={`detail-tab${activeTab === "members" ? " active" : ""}`}
              onClick={() => { setActiveTab("members"); trace.event("detail_tab_switch", { tab: "members" }); }}
            >
              Members
            </button>
            {activeConv.conversation_type === "group" && (
              <button
                className={`detail-tab${activeTab === "queue" ? " active" : ""}`}
                onClick={() => { setActiveTab("queue"); trace.event("detail_tab_switch", { tab: "queue" }); }}
              >
                Queue
              </button>
            )}
            {workspaceGitEnabled && (
              <button
                className={`detail-tab${activeTab === workspaceGitDetailTab.id ? " active" : ""}`}
                onClick={() => { setActiveTab(workspaceGitDetailTab.id); trace.event("detail_tab_switch", { tab: workspaceGitDetailTab.id }); }}
              >
                {workspaceGitDetailTab.label}
              </button>
            )}
            <button
              className={`detail-tab${activeTab === "search" ? " active" : ""}`}
              onClick={() => { setActiveTab("search"); trace.event("detail_tab_switch", { tab: "search" }); }}
            >
              Search
            </button>
          </>
        )}
      </div>

      {/* Tab content */}
      <div className="detail-tab-content">
        {/* === Agent DM tabs === */}
        {agentBinding && activeTab === "overview" && (
          <>
            {/* Members */}
            <div className="detail-section">
              <h4>Members ({activeMembers.length})</h4>
              {activeMembers.map((m) => (
                <MemberRow
                  key={m.id}
                  member={m}
                  isSelf={m.id === principal.id}
                  binding={
                    m.isAgent
                      ? runtimeBindings.find(
                          (b) =>
                            b.agent_principal_id === m.id &&
                            b.conversation_id === activeConv.id,
                        ) ??
                        runtimeBindings.find(
                          (b) => b.agent_principal_id === m.id,
                        )
                      : undefined
                  }
                  machine={m.isAgent ? bindingMachineLabel(
                    runtimeBindings.find((b) => b.agent_principal_id === m.id && b.conversation_id === activeConv.id)
                      ?? runtimeBindings.find((b) => b.agent_principal_id === m.id),
                    runtimeHosts,
                  ) : undefined}
                  isGroup={activeConv.conversation_type === "group"}
                  showSkills={agentSkillsEnabled}
                />
              ))}
            </div>
          </>
        )}

        {agentBinding && activeTab === "config" && (
          <AgentConfigEditor
            binding={agentBinding}
            sessionToken={sessionToken}
          />
        )}

        {agentSkillsEnabled && agentBinding && activeTab === agentSkillsDetailTab.id && (
          <AgentSkillsPanel binding={agentBinding} />
        )}

        {/* === Group tabs === */}
        {!agentBinding && activeTab === "members" && (
          <div className="detail-section">
            <div className="detail-section-header">
              <h4>Members ({activeMembers.length})</h4>
              {activeConv.conversation_type === "group" && (
                <button
                  onClick={() => { setShowAddMember(!showAddMember); setAddMemberSearch(""); }}
                  title="Add member"
                  aria-label="Add member"
                  className="detail-add-member-btn"
                >
                  +
                </button>
              )}
            </div>
            {activeMembers.map((m) => (
              <MemberRow
                key={m.id}
                member={m}
                isSelf={m.id === principal.id}
                binding={
                  m.isAgent
                    ? runtimeBindings.find(
                        (b) =>
                          b.agent_principal_id === m.id &&
                          b.conversation_id === activeConv.id,
                      ) ??
                      runtimeBindings.find(
                        (b) => b.agent_principal_id === m.id,
                      )
                    : undefined
                }
                machine={m.isAgent ? bindingMachineLabel(
                  runtimeBindings.find((b) => b.agent_principal_id === m.id && b.conversation_id === activeConv.id)
                    ?? runtimeBindings.find((b) => b.agent_principal_id === m.id),
                  runtimeHosts,
                ) : undefined}
                isGroup={activeConv.conversation_type === "group"}
                showSkills={agentSkillsEnabled}
                canRemove={
                  activeConv.conversation_type === "group" &&
                  m.id !== activeConv.creator_id
                }
                onRemove={handleRemoveMember}
              />
            ))}

            {/* Add member panel */}
            {showAddMember && activeConv.conversation_type === "group" && (
              <div className="detail-add-member-panel">
                <input
                  type="text"
                  placeholder="Search agents…"
                  value={addMemberSearch}
                  onChange={(e) => setAddMemberSearch(e.target.value)}
                  autoFocus
                  className="detail-input"
                />
                {addError && (
                  <div className="detail-inline-error">{addError}</div>
                )}
                <div className="detail-add-member-list">
                  {filteredAvailable.length === 0 && (
                    <div className="detail-inline-empty">
                      {availableAgents.length === 0 ? "All agents are already in this group" : "No matching agents"}
                    </div>
                  )}
                  {filteredAvailable.map((a) => (
                    <button
                      type="button"
                      key={a.id}
                      className="detail-member-option"
                      disabled={addingIds.has(a.id)}
                      onClick={() => handleAddMember(a.id)}
                    >
                      <Avatar name={a.name} size="tiny" />
                      <span className="detail-member-name">{a.name}</span>
                      <span className="agent-badge">AI</span>
                      <span className="detail-member-action">
                        {addingIds.has(a.id) ? "Adding…" : "+ Add"}
                      </span>
                    </button>
                  ))}
                </div>
              </div>
            )}
          </div>
        )}

        {!agentBinding && activeConv.conversation_type === "group" && activeTab === "queue" && (
          <RuntimeStatusPanel
            conversationId={activeConv.id}
            sessionToken={sessionToken}
          />
        )}

        {workspaceGitEnabled && !agentBinding && activeTab === workspaceGitDetailTab.id && (
          <WorkspaceGitPanel runtimeBindings={runtimeBindings} workspaceId={activeConv.workspace_id} />
        )}

        {!agentBinding && activeTab === "search" && (
          <div className="detail-section">
            <input
              type="text"
              placeholder="Search messages…"
              value={searchQuery}
              onChange={(e) => onSearchInput(e.target.value)}
              autoFocus
              className="detail-input"
            />
            {searchLoading && (
              <div className="detail-inline-empty">Searching…</div>
            )}
            {searchResults.length > 0 && (
              <div className="detail-search-results">
                {searchResults.map((r) => {
                  const senderName = principalName(principal, agents, r.sender_id);
                  const q = searchQuery.toLowerCase();
                  const idx = r.content.toLowerCase().indexOf(q);
                  const previewStart = Math.max(0, idx - 30);
                  const previewEnd = Math.min(r.content.length, idx + q.length + 50);
                  const before = (previewStart > 0 ? "…" : "") + r.content.slice(previewStart, idx);
                  const match = r.content.slice(idx, idx + q.length);
                  const after = r.content.slice(idx + q.length, previewEnd) + (previewEnd < r.content.length ? "…" : "");

                  return (
                    <button
                      key={r.message_id}
                      onClick={() => onSearchResultClick(r.conversation_id)}
                      className="detail-search-result"
                    >
                      <div className="detail-search-result-head">
                        <span className="detail-search-result-sender">{senderName}</span>
                        <span className="detail-search-result-date">
                          {new Date(r.created_at).toLocaleDateString("en-US", { month: "short", day: "numeric" })}
                        </span>
                      </div>
                      <div className="detail-search-result-snippet">
                        {before}<mark>{match}</mark>{after}
                      </div>
                    </button>
                  );
                })}
              </div>
            )}
            {!searchLoading && searchQuery.trim() && searchResults.length === 0 && (
              <EmptyState inline description="No messages match your search." />
            )}
          </div>
        )}
      </div>
    </aside>
  );
}
