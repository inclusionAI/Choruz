"use client";

import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";

const TerminalView = lazy(() =>
  import("../runtime/terminal-view").then((m) => ({ default: m.TerminalView })),
);

import { trace, endTrace, initTrace, startInteractionTracking } from "../../lib/api/choruz-trace";
import { sanitizeTelemetryData } from "../../lib/api/telemetry-sanitize";
import { mergePreviewIntoMessages, appendIncrementalMessages, mergeFetchedMessages, maxCachedSeq, messagesMissingFromPrevious, upsertConfirmedMessage } from "../../lib/messages/messages";
import { persistMessages, loadAllCachedMessages } from "../../lib/messages/message-db";
import type { Principal, Conversation, ChatMessage, MessagePage, ConsoleSnapshot, RuntimeBindingInfo, DashboardBootstrap, DashboardSyncChange, ChannelTask, PatchChannelTaskRequest, Company } from "../../lib/api/choruz-types";
import { principalName, conversationDisplayName, isAgent } from "../../lib/api/principals";
import { agentPeerId, bindingMachineLabel, findAgentDmBinding, findTerminalBinding, openTerminalBindings as terminalBindingsForOpenTabs } from "../../lib/terminal/terminal-bindings";
import { mentionedAgentIds as mentionedAgentIdsIn } from "../../lib/messages/mentions";
import { useConversationFlags } from "../../hooks/use-conversation-flags";
import { useCompanyManagement } from "../../hooks/use-company-management";
import { ResizeHandle } from "../ui/resize-handle";
import { usePanelResize } from "../../hooks/use-panel-resize";
import { useEdgeSwipe } from "../../hooks/use-edge-swipe";
import { useMessageSearch } from "../../hooks/use-message-search";
import { useThinkingAgents } from "../../hooks/use-thinking-agents";
import { thinkingMarkerClearIds } from "../../lib/messages/thinking";
import { Sidebar } from "./sidebar";
import { MessageList } from "./message-list";
import { ChatInput, type ReplyTo } from "./chat-input";
import { ChatHeader } from "./chat-header";
import { ChannelConversationTabs } from "./channel-conversation-tabs";
import { ChannelTaskCreateModal } from "../channel-tasks/channel-task-create-modal";
import { ChatModals } from "./chat-modals";
import { MessagesSquare, Hash, FileCode2, X } from "lucide-react";
import { FileEditor } from "../workspace/file-editor";
import { useChatWebSocket } from "../../hooks/use-chat-web-socket";
import { usePixelWorldStore, emitPixelWorldEvent } from "../pixel-world/pixel-world-store";
import { dedupePrincipals, visibleChannelTaskAssignees as resolveVisibleChannelTaskAssignees } from "../../lib/channel-tasks/channel-task-assignees";
import {
  canCreateTaskFromMessage,
  createMessageTaskIdempotencyKey,
  submitChannelTaskCreateFromMessage,
  type ChannelTaskCreateDraft,
} from "../../lib/channel-tasks/channel-task-create";
import { KanbanBoard, kanbanConversationTab } from "../../plugins/kanban/client";
import { pixelWorldSidebarAction } from "../../plugins/pixel-world/client";
import { remoteSshSidebarAction, RemoteSshModal } from "../../plugins/remote-ssh/client";
import { remoteControlSidebarAction, RemoteControlModal, RuntimeHostsModal } from "../../plugins/remote-control/client";
import { ImportWorkspaceSessionsModal } from "../agents/import-workspace-sessions-modal";
import type { RuntimeHost } from "../../lib/remote/remote-control";
import { resolveClientPluginIds } from "../../plugins/registry";
import {
  mergeChannelTaskList,
  replaceChannelTaskList,
  replaceChannelTask,
  applyChannelTaskCreateResponse,
  restoreChannelTaskAfterFailedMutation,
} from "../../lib/channel-tasks/channel-task-reconciliation";
import {
  createChannelTaskFromMessage,
  fetchChannelTasks,
  fetchConversationMessage,
  fetchThread,
  listRuntimeHosts,
  markThreadViewed,
  patchChannelTask,
  apiFetch,
} from "../../lib/api/choruz-api";
import { collectMissingQuoteTargets, type QuotedMessage } from "../../lib/messages/quotes";
import {
  OPTIMISTIC_SERVER_SEQ,
  mergeThreadReplies,
  partitionThreadMessages,
  resolveThreadRoot,
  rollbackOptimisticMessage,
} from "../../lib/messages/threads";
import {
  applyLocallyViewed,
  buildUnreadMap,
  clearConversationUnread,
  createUnreadCommitGate,
  registerLocallyViewed,
  wsUnreadEffect,
  type LocallyViewedRegistry,
  type UnreadEntry,
  type UnreadRow,
} from "../../lib/messages/thread-unreads";
import { ThreadPanel } from "./thread-panel";
import type { ThreadRollupInfo } from "./message-bubble";
import { EmptyState } from "../ui/empty-state";
import { transportFetch } from "../../lib/api/transport";

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

type ChatAppProps = {
  initialSnapshot: ConsoleSnapshot;
  sessionToken: string;
  runtimeBindings: RuntimeBindingInfo[];
  initialCompanies?: Company[];
  gatewayBaseUrl?: string;
  initialActiveConversationId?: string | null;
  initialActiveConversationView?: "chat" | "tasks";
  initialMessageActionsOpen?: boolean;
  initialSyncCursor?: number;
  initialBootstrapNextCursor?: string | null;
  initialBootstrapHasMore?: boolean;
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const PIXEL_WORLD_OPEN_KEY = "choruz_pixel_world_open";
// Threads: coalescing window for the streaming-path /v1/unreads re-fetch
// and the open-panel read-receipt POST. One knob: the receipt throttle
// resolves into the debounced refresh, so the two windows are defined in
// terms of each other — tune UNREADS_REFRESH_DEBOUNCE_MS and both move.
const UNREADS_REFRESH_DEBOUNCE_MS = 800;
const THREAD_VIEW_RECEIPT_THROTTLE_MS = UNREADS_REFRESH_DEBOUNCE_MS;
// How long the "thinking…" bubble stays up before auto-clearing if no reply
// arrives. This is UX-only — it doesn't bound the agent's real work, which
// is governed by executor_timeout_secs on the backend (24h). 120s comfortably
// covers most LLM responses including ones that read files / call tools /
// chain multiple reasoning steps; shorter values (e.g. 15s, which is what
// hsz0403 originally shipped) cause the bubble to disappear while the agent
// is legitimately still thinking.
const THINKING_AGENT_TTL_MS = 120_000;
const MAX_ATTACHMENT_BYTES = 5 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Backend trace id stamped on an agent reply, when the writer recorded one. */
function messageTraceId(msg: ChatMessage): string | undefined {
  const value = msg.metadata?.trace_id;
  return typeof value === "string" ? value : undefined;
}

// ---------------------------------------------------------------------------
// Component: ChatApp
// ---------------------------------------------------------------------------

export function ChatApp({ initialSnapshot, sessionToken, runtimeBindings: initialBindings, initialCompanies = [], gatewayBaseUrl, initialActiveConversationId = null, initialActiveConversationView = "chat", initialMessageActionsOpen = false, initialSyncCursor = 0, initialBootstrapNextCursor = null, initialBootstrapHasMore = false }: ChatAppProps) {
  // ---- State ----
  const principal = initialSnapshot.principal;
  const [knownPrincipals, setKnownPrincipals] = useState<Principal[]>(
    mergeKnownPrincipals(initialSnapshot.principals ?? [], [
      initialSnapshot.principal,
      ...initialSnapshot.agents,
    ]),
  );
  const [conversations, setConversations] = useState(initialSnapshot.conversations);
  const [bootstrapNextCursor, setBootstrapNextCursor] = useState(initialBootstrapNextCursor);
  const [bootstrapHasMore, setBootstrapHasMore] = useState(initialBootstrapHasMore);
  const [loadingMoreConversations, setLoadingMoreConversations] = useState(false);
  const [messagesByConv, setMessagesByConv] = useState(initialSnapshot.messages_by_conversation);
  const [messagePageState, setMessagePageState] = useState<Record<string, {
    hasMoreBefore: boolean;
    loadingBefore: boolean;
  }>>({});
  const loadingOlderConversationIdsRef = useRef(new Set<string>());
  const [agents, setAgents] = useState(initialSnapshot.agents);
  const [hostPlugins, setHostPlugins] = useState(initialSnapshot.plugins ?? []);
  const clientPluginIds = useMemo(() => resolveClientPluginIds(hostPlugins), [hostPlugins]);
  const kanbanEnabled = clientPluginIds.has("kanban");
  const pixelWorldEnabled = clientPluginIds.has("pixel-world");
  const workspaceGitEnabled = clientPluginIds.has("workspace-git");
  const remoteSshEnabled = clientPluginIds.has("remote-ssh");
  const remoteControlEnabled = clientPluginIds.has("remote-control");
  const agentSkillsEnabled = clientPluginIds.has("agent-skills");
  const mathcodeEnabled = clientPluginIds.has("mathcode");
  const [channelTasksByConv, setChannelTasksByConv] = useState<Record<string, ChannelTask[]>>({});
  const [channelTaskLoadErrors, setChannelTaskLoadErrors] = useState<Record<string, string | null>>({});
  const [loadingChannelTaskConvIds, setLoadingChannelTaskConvIds] = useState<Set<string>>(new Set());
  const [mutatingChannelTaskIds, setMutatingChannelTaskIds] = useState<Set<string>>(new Set());
  const [channelTaskRefetchConvIds, setChannelTaskRefetchConvIds] = useState<string[]>([]);
  const [runtimeBindings, setRuntimeBindings] = useState(initialBindings);
  const [activeConvId, setActiveConvId] = useState<string | null>(initialActiveConversationId);
  const [showDetail, setShowDetail] = useState(false);
  const [showPixelWorld, setShowPixelWorld] = useState(false);
  const [pixelWorldPreferenceLoaded, setPixelWorldPreferenceLoaded] = useState(false);
  const [showSidebar, setShowSidebar] = useState(false);
  const sidebarResize = usePanelResize({ storageKey: "choruz_sidebar_width", initial: 320, min: 240, max: 480, anchor: "left" });
  const detailResize = usePanelResize({ storageKey: "choruz_detail_width", initial: 320, min: 280, max: 600, anchor: "right" });
  // ---- Analytics ----
  const trackEvent = useCallback((event: string, data?: Record<string, unknown>) => {
    transportFetch("/api/analytics", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ event, data: sanitizeTelemetryData(data), timestamp: new Date().toISOString() }),
    }).catch(() => {});
  }, []);

  const flags = useConversationFlags({
    initial: {
      pinned: initialSnapshot.pinned_conversations ?? [],
      archived: initialSnapshot.archived_conversations ?? [],
      hidden: initialSnapshot.hidden_conversations ?? [],
    },
    conversations,
    sessionToken,
    trackEvent,
  });

  // Initialize tracing on mount
  useEffect(() => {
    initTrace(sessionToken, gatewayBaseUrl ?? "");
    startInteractionTracking();
  }, [sessionToken, gatewayBaseUrl]);

  useEffect(() => {
    if (!pixelWorldPreferenceLoaded) return;
    try {
      localStorage.setItem(PIXEL_WORLD_OPEN_KEY, String(showPixelWorld));
    } catch { /* ignore */ }
  }, [pixelWorldPreferenceLoaded, showPixelWorld]);

  useEffect(() => {
    if (!pixelWorldEnabled) setShowPixelWorld(false);
  }, [pixelWorldEnabled]);

  // ---- IndexedDB: load persisted messages on cold start ----
  useEffect(() => {
    const convIds = initialSnapshot.conversations.map((c) => c.id);
    if (convIds.length === 0) return;
    loadAllCachedMessages(convIds).then((cached) => {
      if (Object.keys(cached).length === 0) return;
      // Merge IDB cache into state without overwriting messages already
      // delivered by bootstrap or WebSocket during the async IDB load.
      setMessagesByConv((prev) => {
        const merged = { ...prev };
        let changed = false;
        for (const [cid, idbMsgs] of Object.entries(cached)) {
          const existing = prev[cid] ?? [];
          if (idbMsgs.length > existing.length) {
            merged[cid] = idbMsgs;
            changed = true;
          }
        }
        return changed ? merged : prev;
      });
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Server-side unread tracking (Mattermost pattern). `threadUnread` is the
  // server-canonical COUNT(DISTINCT thread) with unread replies — cleared
  // per thread via POST /threads/{root}/view, NOT by viewing the
  // conversation. Seam logic lives in lib/messages/thread-unreads.ts (unit-tested).
  const [unreads, setUnreads] = useState<Record<string, UnreadEntry>>(
    () => buildUnreadMap(initialSnapshot.unreads ?? []),
  );
  // Conversations viewed locally whose POST /view may not be reflected in
  // an in-flight unreads response yet. Seq-aware registry: consumed by the
  // first response whose REQUEST post-dates the view; an older in-flight
  // response is papered over without eating the protection. Semantics
  // pinned in lib/messages/thread-unreads.test.ts (the R5→R6 Critical regression).
  const locallyViewedConversationIdsRef = useRef<LocallyViewedRegistry>(new Map());
  // Claim/commit ordering shared by every on-demand /v1/unreads writer:
  // claim a seq at dispatch, commit only newer-than-last-COMMIT. The gate semantics (burned claims must
  // not veto concurrent successful commits) are pinned in
  // lib/messages/thread-unreads.test.ts — do not inline these refs back together.
  const unreadCommitGateRef = useRef(createUnreadCommitGate());

  // Single commit point for server unread rows: commit-ordering + the
  // locally-viewed overlay. All writers MUST go through this.
  const commitUnreadRows = useCallback(
    (rows: UnreadRow[], requestSeq: number) => {
      if (!unreadCommitGateRef.current.tryCommit(requestSeq)) return;
      setUnreads(
        applyLocallyViewed(buildUnreadMap(rows), locallyViewedConversationIdsRef.current, requestSeq),
      );
    },
    [],
  );

  const refreshUnreads = useCallback(() => {
    const requestSeq = unreadCommitGateRef.current.claim();
    apiFetch<UnreadRow[]>("/v1/unreads", sessionToken)
      .then((data) => commitUnreadRows(data, requestSeq))
      .catch(() => {});
  }, [sessionToken, commitUnreadRows]);

  // Debounced variant for streaming paths (WS thread replies, open-panel
  // receipts): the thread counter can't be maintained locally (it counts
  // DISTINCT threads server-side), so these paths re-fetch — coalesced so
  // a burst of agent replies costs one request, not N.
  const refreshUnreadsTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const refreshUnreadsDebounced = useCallback(() => {
    if (refreshUnreadsTimerRef.current) clearTimeout(refreshUnreadsTimerRef.current);
    refreshUnreadsTimerRef.current = setTimeout(() => {
      refreshUnreadsTimerRef.current = null;
      refreshUnreads();
    }, UNREADS_REFRESH_DEBOUNCE_MS);
  }, [refreshUnreads]);
  useEffect(() => {
    return () => {
      if (refreshUnreadsTimerRef.current) clearTimeout(refreshUnreadsTimerRef.current);
    };
  }, []);

  // ---- IndexedDB: write-through on message state changes ----
  const prevMessagesByConvRef = useRef(messagesByConv);
  useEffect(() => {
    const prev = prevMessagesByConvRef.current;
    prevMessagesByConvRef.current = messagesByConv;
    // Diff: only persist messages not seen in the previous in-memory cache.
    // Full fetches can backfill older messages before a cached preview/WS tail,
    // so this cannot rely on array length or append-only ordering.
    const toPersist: ChatMessage[] = [];
    for (const [cid, msgs] of Object.entries(messagesByConv)) {
      if (msgs !== prev[cid] && msgs.length > 0) {
        const delta = messagesMissingFromPrevious(prev[cid], msgs);
        if (delta.length > 0) {
          toPersist.push(...delta);
        }
      }
    }
    if (toPersist.length > 0) {
      persistMessages(toPersist);
    }
  }, [messagesByConv]);

  // Company state
  const {
    companies,
    replaceCompanies,
    addCompany,
    toggleAgentsActive,
    setMultiHarnessAccounts,
    archiveCompany,
    unarchiveCompany,
    deleteCompany,
    renameCompany,
    changeCompanyWorkspace,
  } = useCompanyManagement({ initialCompanies, sessionToken, actorId: principal.id });
  const [activeCompanyId, setActiveCompanyId] = useState<string | null>(
    initialCompanies.find((company) => company.slug === "default")?.id ?? initialCompanies[0]?.id ?? null,
  );
  const multiHarnessAccounts = companies.find((company) => company.id === activeCompanyId)?.multi_harness_accounts ?? false;
  const activeConversationByCompanyRef = useRef<Map<string, string>>(new Map());
  const [showCreateCompany, setShowCreateCompany] = useState(false);

  // Modals
  const [showCreateGroup, setShowCreateGroup] = useState(false);
  const [showCreateAgent, setShowCreateAgent] = useState(false);
  const [showHarnessAccounts, setShowHarnessAccounts] = useState(false);
  const [showServers, setShowServers] = useState(false);
  const [showRemoteControl, setShowRemoteControl] = useState(false);
  const [machinesCompanyId, setMachinesCompanyId] = useState<string | null>(null);
  const [runtimeHosts, setRuntimeHosts] = useState<RuntimeHost[]>([]);
  const [showWorkspaceSessionImport, setShowWorkspaceSessionImport] = useState(false);
  const closeWorkspaceSessionImport = useCallback(() => {
    setShowWorkspaceSessionImport(false);
  }, []);

  useEffect(() => {
    let cancelled = false;
    if (!remoteControlEnabled || !activeCompanyId) {
      setRuntimeHosts([]);
      return;
    }
    void listRuntimeHosts(sessionToken, activeCompanyId)
      .then((hosts) => { if (!cancelled) setRuntimeHosts(hosts); })
      .catch(() => { if (!cancelled) setRuntimeHosts([]); });
    return () => { cancelled = true; };
  }, [activeCompanyId, remoteControlEnabled, sessionToken]);

  // Which agents show a "thinking…" bubble.
  const {
    thinkingAgents,
    markThinking: markAgentsThinking,
    clearThinking: clearThinkingAgentsByIds,
  } = useThinkingAgents(THINKING_AGENT_TTL_MS);

  // Quote-reply state
  const [replyTo, setReplyTo] = useState<ReplyTo | null>(null);
  // Thread side-panel state: the open thread's root message id (null = closed)
  const [openThreadRootId, setOpenThreadRootId] = useState<string | null>(null);
  const [threadLoading, setThreadLoading] = useState(false);
  const [threadError, setThreadError] = useState<string | null>(null);

  // Quote-reply previews (WeChat/Feishu-style): originals referenced by a
  // reply but outside the loaded history window, fetched on demand and
  // kept in a dedicated store — NOT messagesByConv, where an isolated old
  // message would surface mid-timeline. Ids are globally unique (UUIDv7),
  // so one flat map serves every conversation; "missing" pins a 404 so a
  // deleted original isn't re-fetched on every render.
  const [quotedMessages, setQuotedMessages] = useState<Map<string, QuotedMessage>>(new Map());
  const quoteFetchInFlightRef = useRef(new Set<string>());
  const [createTaskDraft, setCreateTaskDraft] = useState<ChannelTaskCreateDraft | null>(null);
  const [createTaskError, setCreateTaskError] = useState<string | null>(null);
  const [creatingTaskFromMessage, setCreatingTaskFromMessage] = useState(false);
  const activeCreateTaskAttemptRef = useRef<string | null>(null);

  // Unified tab state: conversations + files
  type Tab = { type: 'conv'; convId: string } | { type: 'file'; path: string; workspaceId: string | null; dirty: boolean };
  const [openTabs, setOpenTabs] = useState<Tab[]>(
    initialActiveConversationId
      ? [{ type: "conv" as const, convId: initialActiveConversationId }]
      : [],
  );
  const [activeTabId, setActiveTabId] = useState<string | null>(initialActiveConversationId); // convId or filePath
  const [activeConversationView, setActiveConversationView] = useState<"chat" | "tasks">(initialActiveConversationView);

  const selectCompany = useCallback((companyId: string) => {
    const visibleConversationIds = new Set(
      conversations
        .filter((conversation) => conversation.workspace_id === companyId)
        .map((conversation) => conversation.id),
    );
    if (activeCompanyId && activeConvId) {
      activeConversationByCompanyRef.current.set(activeCompanyId, activeConvId);
    }
    const rememberedConversationId = activeConversationByCompanyRef.current.get(companyId);
    const nextConversationId = rememberedConversationId && visibleConversationIds.has(rememberedConversationId)
      ? rememberedConversationId
      : null;
    setActiveCompanyId(companyId);
    setOpenTabs((previous) => {
      const visibleTabs = previous.filter((tab) =>
        tab.type === "conv"
          ? visibleConversationIds.has(tab.convId)
          : tab.workspaceId === companyId
      );
      if (!nextConversationId || visibleTabs.some((tab) => tab.type === "conv" && tab.convId === nextConversationId)) {
        return visibleTabs;
      }
      return [...visibleTabs, { type: "conv" as const, convId: nextConversationId }];
    });
    setActiveConvId(nextConversationId);
    setActiveTabId(nextConversationId);
    if (nextConversationId) {
      try { localStorage.setItem("choruz_active_conv", nextConversationId); } catch {}
    }
    setReplyTo(null);
    setOpenThreadRootId(null);
  }, [activeCompanyId, activeConvId, conversations]);

  // Deleting the active company moves to the first remaining one.
  const handleDeleteCompany = useCallback(async (companyId: string) => {
    await deleteCompany(companyId);
    if (activeCompanyId !== companyId) return;
    const next = companies.find((company) => company.id !== companyId);
    if (next) selectCompany(next.id);
    else setActiveCompanyId(null);
  }, [activeCompanyId, companies, deleteCompany, selectCompany]);

  // Helper to get tab ID
  const tabId = (tab: Tab) => tab.type === 'conv' ? tab.convId : tab.path;

  // Open a file tab
  const openFile = useCallback((path: string) => {
    trace.event("open_file", { path, fileName: path.split('/').pop() });
    setOpenTabs(prev => {
      if (prev.some(t => t.type === 'file' && t.path === path)) return prev;
      return [...prev, { type: 'file', path, workspaceId: activeCompanyId, dirty: false }];
    });
    setActiveTabId(path);
  }, [activeCompanyId]);

  // Close any tab
  const closeTab = useCallback((id: string) => {
    setOpenTabs(prev => {
      const filtered = prev.filter(t => tabId(t) !== id);
      return filtered;
    });
    setActiveTabId(prev => {
      if (prev !== id) return prev;
      // Switch to most recent remaining tab, or the active conversation
      return activeConvId;
    });
  }, [activeConvId]);

  // Stable ref for activeConvId so WS handler can read it without being
  // a dependency (which would thrash the callback on every selection).
  const activeConvIdRef = useRef(activeConvId);
  activeConvIdRef.current = activeConvId;
  const channelTasksByConvRef = useRef(channelTasksByConv);
  channelTasksByConvRef.current = channelTasksByConv;
  const queueChannelTaskRefetch = useCallback((conversationId?: string | null) => {
    if (!kanbanEnabled) return;
    setChannelTaskRefetchConvIds((prev) => {
      const next = new Set(prev);
      if (conversationId) {
        next.add(conversationId);
      } else {
        if (activeConvIdRef.current) {
          next.add(activeConvIdRef.current);
        }
        for (const loadedConversationId of Object.keys(channelTasksByConvRef.current)) {
          next.add(loadedConversationId);
        }
      }
      return [...next];
    });
  }, [kanbanEnabled]);
  const refreshBootstrapRef = useRef<() => Promise<void>>(async () => {});

  // ---- WebSocket real-time push ----
  const handleRealtimeMessage = useCallback(
    (newMsg: ChatMessage) => {
      const convId = newMsg.conversation_id;

      setMessagesByConv((prev) => {
        const existing = prev[convId] ?? [];
        const updated = upsertConfirmedMessage(existing, newMsg);
        return updated === existing ? prev : { ...prev, [convId]: updated };
      });

      // Local unread bookkeeping (server-side counters are canonical; this
      // keeps badges responsive before the next /v1/unreads fetch). The
      // effect mirrors the server's gates — see wsUnreadEffect: quiet
      // thread replies skip the conversation badge, and the THREAD counter
      // is never bumped locally (it counts DISTINCT threads server-side;
      // a per-message increment would drift), only re-fetched debounced.
      if (newMsg.sender_id !== principal.id) {
        const effect = wsUnreadEffect(newMsg);
        // Conversation badge: only for non-active conversations (the
        // active one is being read right now).
        if (convId !== activeConvIdRef.current && effect.bumpConversationUnread) {
          setUnreads((prev) => {
            const cur = prev[convId] ?? { unread: 0, mentions: 0 };
            return { ...prev, [convId]: { ...cur, unread: cur.unread + 1 } };
          });
        }
        // Thread badge: refresh REGARDLESS of which conversation is
        // active — a quiet reply in the ACTIVE conversation never hits the
        // main timeline, so without this the thread badge would sit stale
        // until the debounced canonical unread refresh.
        if (effect.refreshThreadUnread) {
          refreshUnreadsDebounced();
        }
      }

      clearThinkingAgentsByIds(thinkingMarkerClearIds([newMsg]));

      if (isAgent(agents, newMsg.sender_id)) {
        const agentObj = agents.find((a) => a.id === newMsg.sender_id);
        // Pull the backend-originated trace_id off the reply's metadata —
        // writer.rs stamps it when committing the reply event. Without this
        // the FE agent_reply logs would be joined to whatever FE trace
        // happens to be active at receipt time (a different user action,
        // or worse, a stale one), instead of to the message that actually
        // caused this reply.
        const backendTraceId = messageTraceId(newMsg);
        trace.event("agent_reply", {
          agent_id: newMsg.sender_id,
          agent_name: agentObj?.name,
          conversation_id: convId,
          content_len: newMsg.content.length,
          event_type: "message.created",
          source: "ws",
          backend_trace_id: backendTraceId ?? null,
        });
      }

      if (pixelWorldEnabled) {
        usePixelWorldStore.getState().handleMessage(newMsg.sender_id, convId);
        for (const agent of agents) {
          if (newMsg.content.toLowerCase().includes(`@${agent.name.toLowerCase()}`)) {
            usePixelWorldStore.getState().handleMention(agent.id);
          }
        }
      }
    },
    [agents, principal.id, clearThinkingAgentsByIds, pixelWorldEnabled, refreshUnreadsDebounced],
  );
  const handleWsParseError = useCallback(() => {
    queueChannelTaskRefetch(null);
  }, [queueChannelTaskRefetch]);

  const handleSyncChanges = useCallback(async (changes: DashboardSyncChange[]) => {
    const messagesToPersist: ChatMessage[] = [];
    let refreshBootstrap = false;
    let shouldRefreshUnreads = false;
    const changedBindingIds = new Set<string>();
    const deletedBindingIds = new Set<string>();

    for (const change of changes) {
      if (change.event_type === "message.created") {
        const payload = change.payload;
        const conversationId = typeof payload.conversation_id === "string"
          ? payload.conversation_id
          : change.conversation_id;
        const messageId = typeof payload.message_id === "string" ? payload.message_id : change.entity_id;
        const senderId = typeof payload.sender_id === "string" ? payload.sender_id : null;
        const content = typeof payload.content === "string" ? payload.content : null;
        const serverSeq = typeof payload.server_seq === "number" ? payload.server_seq : null;
        if (!conversationId || !senderId || content === null || serverSeq === null) {
          throw new Error(`invalid message.created sync payload at cursor ${change.cursor}`);
        }
        const metadata = payload.metadata && typeof payload.metadata === "object" && !Array.isArray(payload.metadata)
          ? payload.metadata as Record<string, unknown>
          : {};
        const message: ChatMessage = {
          id: messageId,
          workspace_id: typeof payload.workspace_id === "string" ? payload.workspace_id : "",
          conversation_id: conversationId,
          sender_id: senderId,
          content,
          content_type: typeof payload.content_type === "string" ? payload.content_type : "text",
          metadata,
          edited_at: null,
          edited_by: null,
          server_seq: serverSeq,
          idempotency_key: typeof payload.client_msg_id === "string" ? payload.client_msg_id : "",
          created_at: change.created_at,
        };
        handleRealtimeMessage(message);
        messagesToPersist.push(message);
        continue;
      }

      const conversationId = change.conversation_id;
      if (change.event_type === "conversation.deleted" && conversationId) {
        setConversations((current) => current.filter((conversation) => conversation.id !== conversationId));
        setMessagesByConv((current) => {
          if (!(conversationId in current)) return current;
          const next = { ...current };
          delete next[conversationId];
          return next;
        });
      } else if (flags.applyChange(change)) {
        // Pin / archive / hidden flag; absorbed by the hook.
      } else if (change.event_type === "conversation.read_state_changed" || change.event_type === "thread.read_state_changed") {
        shouldRefreshUnreads = true;
      } else if (change.entity_type === "channel_task") {
        queueChannelTaskRefetch(conversationId);
      } else if (change.entity_type === "runtime_binding") {
        // The trigger fires on every binding column that matters, including
        // state and config_json changes during a turn, so re-read that one
        // binding rather than the whole snapshot.
        if (change.event_type === "runtime_binding.deleted") {
          changedBindingIds.delete(change.entity_id);
          deletedBindingIds.add(change.entity_id);
        } else {
          deletedBindingIds.delete(change.entity_id);
          changedBindingIds.add(change.entity_id);
        }
      } else {
        refreshBootstrap = true;
      }
    }

    if (changedBindingIds.size > 0 || deletedBindingIds.size > 0) {
      const refreshed = await Promise.all(
        [...changedBindingIds].map(async (bindingId) => {
          try {
            return await apiFetch<RuntimeBindingInfo>(`/v1/runtime/bindings/${bindingId}`, sessionToken);
          } catch (err) {
            // Gone or no longer visible to this person: drop it like a delete.
            trace.event("runtime_binding_refetch_failed", { bindingId, error: String(err) });
            deletedBindingIds.add(bindingId);
            return null;
          }
        }),
      );
      setRuntimeBindings((current) => {
        const byId = new Map(current.map((binding) => [binding.id, binding]));
        for (const bindingId of deletedBindingIds) byId.delete(bindingId);
        for (const binding of refreshed) if (binding) byId.set(binding.id, binding);
        return [...byId.values()];
      });
    }

    if (messagesToPersist.length > 0) await persistMessages(messagesToPersist);
    if (shouldRefreshUnreads) refreshUnreadsDebounced();
    if (refreshBootstrap) await refreshBootstrapRef.current();
  }, [flags.applyChange, handleRealtimeMessage, queueChannelTaskRefetch, refreshUnreadsDebounced, sessionToken]);

  const { status: wsStatus } = useChatWebSocket(
    principal.id,
    initialSyncCursor,
    handleSyncChanges,
    handleWsParseError,
  );

  // Refs
  // Stable ref for callbacks that must read the latest cache without
  // rebuilding their identity on every message.
  const messagesByConvRef = useRef(messagesByConv);
  messagesByConvRef.current = messagesByConv;
  const loadedMessageHistoryRef = useRef(new Set<string>());

  // Restore client-only state from localStorage (avoids hydration mismatch)
  useEffect(() => {
    try {
      setShowPixelWorld(localStorage.getItem(PIXEL_WORLD_OPEN_KEY) === "true");
    } catch { /* ignore */ }
    setPixelWorldPreferenceLoaded(true);

    // Prefer an explicit dashboard deep link, then fall back to the last
    // locally selected conversation.
    try {
      const requested = new URLSearchParams(window.location.search).get("conversationId");
      const hiddenConversationIds = new Set(
        (initialSnapshot.hidden_conversations ?? []).map((hidden) => hidden.conversation_id),
      );
      const initialExists =
        initialActiveConversationId &&
        !hiddenConversationIds.has(initialActiveConversationId) &&
        initialSnapshot.conversations.some((c) => c.id === initialActiveConversationId);
      const requestedExists = requested &&
        !hiddenConversationIds.has(requested) &&
        initialSnapshot.conversations.some((c) => c.id === requested);
      const saved = localStorage.getItem("choruz_active_conv");
      if (saved && hiddenConversationIds.has(saved)) {
        localStorage.removeItem("choruz_active_conv");
      }
      const savedExists = saved &&
        !hiddenConversationIds.has(saved) &&
        initialSnapshot.conversations.some((c) => c.id === saved);
      const restored = initialExists ? initialActiveConversationId : requestedExists ? requested : savedExists ? saved : null;

      if (restored) {
        setActiveConvId(restored);
        setActiveTabId(restored);
        setOpenTabs([{ type: 'conv' as const, convId: restored }]);

        try { localStorage.setItem("choruz_active_conv", restored); } catch {}
        apiFetch(`/v1/conversations/${restored}/view`, sessionToken, { method: "POST" }).catch(() => {});
        registerLocallyViewed(
          locallyViewedConversationIdsRef.current,
          restored,
          unreadCommitGateRef.current.claimedSeq(),
        );
        setUnreads((prev) => clearConversationUnread(prev, restored));

        const qs = new URLSearchParams({
          principal_id: principal.id,
          limit: "50",
        });
        apiFetch<MessagePage>(
          `/v1/conversations/${restored}/message-page?${qs}`,
          sessionToken,
        )
          .then((page) => {
            loadedMessageHistoryRef.current.add(restored);
            setMessagePageState((prev) => ({
              ...prev,
              [restored]: { hasMoreBefore: page.has_more, loadingBefore: false },
            }));
            setMessagesByConv((prev) => mergeFetchedMessages(prev, restored, page.messages));
          })
          .catch(() => {});
      }
    } catch {}
  }, []);

  // ---- Derived ----
  const activeConv = useMemo(
    () => conversations.find((c) => c.id === activeConvId) ?? null,
    [conversations, activeConvId],
  );
  const activeFileTab = useMemo(
    () => openTabs.find((t) => t.type === "file" && t.path === activeTabId),
    [openTabs, activeTabId],
  );
  const companyConversations = useMemo(
    () => activeCompanyId ? conversations.filter(c => c.workspace_id === activeCompanyId) : conversations,
    [conversations, activeCompanyId],
  );
  const companyAgents = useMemo(
    () => activeCompanyId ? agents.filter(a => a.workspace_id === activeCompanyId) : agents,
    [agents, activeCompanyId],
  );
  const sidebarPinnedConversations = useMemo(() => {
    const activeConversationIds = new Set(companyConversations.map((conversation) => conversation.id));
    return flags.pinned.filter((pin) => activeConversationIds.has(pin.conversation_id));
  }, [flags.pinned, companyConversations]);
  const sidebarArchivedConversations = useMemo(() => {
    const activeConversationIds = new Set(companyConversations.map((conversation) => conversation.id));
    return flags.archived.filter((archive) =>
      activeConversationIds.has(archive.conversation_id),
    );
  }, [flags.archived, companyConversations]);

  const activeMessages = useMemo(
    () => {
      const msgs = activeConvId ? messagesByConv[activeConvId] ?? [] : [];
      // Ensure messages are sorted by server_seq (arrival order may differ from seq order)
      return [...msgs].sort((a, b) => a.server_seq - b.server_seq);
    },
    [messagesByConv, activeConvId],
  );

  // ---- Threads ----
  // Split the conversation into the main timeline (quiet thread replies
  // hidden) and per-root rollups. Optimistic sends (server_seq = MAX_SAFE_
  // INTEGER) sort last, so they land at the bottom of their thread too.
  const { timeline: timelineMessages, rollups: threadRollupsRaw } = useMemo(
    () => partitionThreadMessages(activeMessages),
    [activeMessages],
  );
  const resolveName = useCallback(
    (id: string) => principalName(principal, agents, id),
    [principal, agents],
  );
  const threadRollups = useMemo(() => {
    const map = new Map<string, ThreadRollupInfo>();
    for (const [rootId, rollup] of threadRollupsRaw) {
      map.set(rootId, {
        replyCount: rollup.replyCount,
        lastReplyAt: rollup.lastReplyAt,
        participantNames: rollup.participantIds.map(resolveName),
      });
    }
    return map;
  }, [threadRollupsRaw, resolveName]);
  const openThreadRoot = useMemo(
    () => (openThreadRootId ? activeMessages.find((m) => m.id === openThreadRootId) ?? null : null),
    [openThreadRootId, activeMessages],
  );
  const openThreadReplies = useMemo(
    () => (openThreadRootId ? threadRollupsRaw.get(openThreadRootId)?.replies ?? [] : []),
    [openThreadRootId, threadRollupsRaw],
  );

  // On-demand quote-target fetch: whenever loaded messages reference an
  // original that isn't loaded, fetched, or already being fetched, pull it
  // from the DB so the quote block shows real content instead of a
  // placeholder. The in-flight ref dedups across re-renders; a 404 is
  // pinned as "missing" (deleted originals are never re-fetched).
  useEffect(() => {
    if (!activeConvId) return;
    const convId = activeConvId;
    const targets = collectMissingQuoteTargets(
      activeMessages,
      quotedMessages,
      quoteFetchInFlightRef.current,
    );
    for (const targetId of targets) {
      quoteFetchInFlightRef.current.add(targetId);
      fetchConversationMessage(sessionToken, convId, targetId)
        .then((original) => {
          setQuotedMessages((prev) => new Map(prev).set(targetId, original));
        })
        .catch((err) => {
          const detail = (err instanceof Error ? err.message : String(err)).toLowerCase();
          if (detail.includes("not found") || detail.includes("404")) {
            setQuotedMessages((prev) => new Map(prev).set(targetId, "missing"));
          }
          // Transient errors: leave unresolved; the in-flight ref is
          // cleared below, so a later render retries.
        })
        .finally(() => {
          quoteFetchInFlightRef.current.delete(targetId);
        });
    }
  }, [activeConvId, activeMessages, quotedMessages, sessionToken]);
  const showChannelTasksTab = kanbanConversationTab.isVisible({
    conversation: activeConv,
    principal,
    agents,
    channelTasksEnabled: kanbanEnabled,
  });
  const visibleChannelTaskAssignees = useMemo(() => {
    return resolveVisibleChannelTaskAssignees({
      conversation: activeConv,
      principals: mergeKnownPrincipals(knownPrincipals, [principal, ...agents]),
    });
  }, [activeConv, knownPrincipals, principal, agents]);
  const canCreateChannelTaskFromActiveMessage = showChannelTasksTab && visibleChannelTaskAssignees.length > 0;

  useEffect(() => {
    if (!showChannelTasksTab && activeConversationView === "tasks") {
      setActiveConversationView("chat");
    }
  }, [showChannelTasksTab, activeConversationView]);

  useEffect(() => {
    setCreateTaskDraft(null);
    setCreateTaskError(null);
    setCreatingTaskFromMessage(false);
    activeCreateTaskAttemptRef.current = null;
  }, [activeConvId]);

  const refreshChannelTasks = useCallback(async (
    conversationId: string,
    options: { replace?: boolean } = {},
  ) => {
    if (!kanbanEnabled) return;
    setLoadingChannelTaskConvIds((prev) => new Set(prev).add(conversationId));
    setChannelTaskLoadErrors((prev) => ({ ...prev, [conversationId]: null }));
    try {
      const tasks = await fetchChannelTasks(sessionToken, conversationId);
      setChannelTasksByConv((prev) => ({
        ...prev,
        [conversationId]: options.replace
          ? replaceChannelTaskList(tasks)
          : mergeChannelTaskList(prev[conversationId] ?? [], tasks),
      }));
    } catch (error) {
      const message = error instanceof Error ? error.message : "Unable to load tasks";
      setChannelTaskLoadErrors((prev) => ({ ...prev, [conversationId]: message }));
    } finally {
      setLoadingChannelTaskConvIds((prev) => {
        const next = new Set(prev);
        next.delete(conversationId);
        return next;
      });
    }
  }, [kanbanEnabled, sessionToken]);

  useEffect(() => {
    if (channelTaskRefetchConvIds.length === 0) return;
    const conversationIds = channelTaskRefetchConvIds;
    setChannelTaskRefetchConvIds([]);
    for (const conversationId of conversationIds) {
      void refreshChannelTasks(conversationId, { replace: true });
    }
  }, [channelTaskRefetchConvIds, refreshChannelTasks]);

  const previousWsStatusRef = useRef(wsStatus);
  useEffect(() => {
    const previousStatus = previousWsStatusRef.current;
    previousWsStatusRef.current = wsStatus;
    if (wsStatus === "connected" && previousStatus !== "connected") {
      queueChannelTaskRefetch(null);
    }
  }, [queueChannelTaskRefetch, wsStatus]);

  useEffect(() => {
    if (!activeConvId || !showChannelTasksTab || activeConversationView !== "tasks") return;
    void refreshChannelTasks(activeConvId);
  }, [activeConvId, activeConversationView, showChannelTasksTab, refreshChannelTasks]);

  const handlePatchChannelTask = useCallback(async (
    taskId: string,
    patch: PatchChannelTaskRequest,
  ) => {
    if (!activeConvId) return;
    const conversationId = activeConvId;
    const previousTask = (channelTasksByConv[conversationId] ?? []).find((task) => task.task_id === taskId);
    setMutatingChannelTaskIds((prev) => new Set(prev).add(taskId));
    setChannelTaskLoadErrors((prev) => ({ ...prev, [conversationId]: null }));
    setChannelTasksByConv((prev) => ({
      ...prev,
      [conversationId]: (prev[conversationId] ?? []).map((task) =>
        task.task_id === taskId ? optimisticChannelTask(task, patch, visibleChannelTaskAssignees) : task,
      ),
    }));
    try {
      const updated = await patchChannelTask(sessionToken, taskId, patch);
      setChannelTasksByConv((prev) => ({
        ...prev,
        [updated.conversation_id]: replaceChannelTask(prev[updated.conversation_id] ?? [], updated),
      }));
    } catch (error) {
      const message = error instanceof Error ? error.message : "Unable to update task";
      trace.event("channel_task_patch_error", {
        convId: conversationId,
        taskId,
        patchKeys: Object.keys(patch),
        error: message,
      });
      setChannelTaskLoadErrors((prev) => ({ ...prev, [conversationId]: message }));
      if (previousTask) {
        setChannelTasksByConv((prev) => ({
          ...prev,
          [conversationId]: restoreChannelTaskAfterFailedMutation(prev[conversationId] ?? [], previousTask),
        }));
      }
    } finally {
      setMutatingChannelTaskIds((prev) => {
        const next = new Set(prev);
        next.delete(taskId);
        return next;
      });
    }
  }, [activeConvId, channelTasksByConv, sessionToken, visibleChannelTaskAssignees]);

  const handleOpenCreateTaskFromMessage = useCallback((message: ChatMessage) => {
    if (!canCreateTaskFromMessage({
      channelTasksVisible: canCreateChannelTaskFromActiveMessage,
      conversation: activeConv,
      message,
      visibleAssignees: visibleChannelTaskAssignees,
    })) {
      return;
    }
    setCreateTaskError(null);
    setCreateTaskDraft({
      message,
      idempotencyKey: createMessageTaskIdempotencyKey(message.id),
    });
  }, [activeConv, canCreateChannelTaskFromActiveMessage, visibleChannelTaskAssignees]);

  const handleSubmitCreateTaskFromMessage = useCallback(async (payload: {
    title: string;
    assigneePrincipalId: string;
    contextLabel: string | null;
  }) => {
    if (!createTaskDraft) return;
    const draft = createTaskDraft;
    const conversationId = draft.message.conversation_id;
    activeCreateTaskAttemptRef.current = draft.idempotencyKey;
    setCreatingTaskFromMessage(true);
    setCreateTaskError(null);
    try {
      const result = await submitChannelTaskCreateFromMessage({
        sessionToken,
        draft,
        payload,
        createTask: createChannelTaskFromMessage,
      });
      if (!result.ok) {
        throw new Error(result.error);
      }
      const task = result.task;
      setChannelTasksByConv((prev) => ({
        ...prev,
        [task.conversation_id]: applyChannelTaskCreateResponse(prev[task.conversation_id] ?? [], task),
      }));
      setCreateTaskDraft((prev) => (prev?.idempotencyKey === draft.idempotencyKey ? null : prev));
      if (activeConvIdRef.current === conversationId) {
        setActiveConversationView("tasks");
      }
    } catch (error) {
      if (activeCreateTaskAttemptRef.current === draft.idempotencyKey) {
        const message = error instanceof Error ? error.message : "Unable to create task";
        trace.event("channel_task_create_from_message_error", {
          convId: conversationId,
          msgId: draft.message.id,
          error: message,
        });
        setCreateTaskError(message);
      }
    } finally {
      if (activeCreateTaskAttemptRef.current === draft.idempotencyKey) {
        activeCreateTaskAttemptRef.current = null;
        setCreatingTaskFromMessage(false);
      }
    }
  }, [createTaskDraft, sessionToken]);

  // ---- On-demand transcript recovery (incremental via after_seq) ----
  const refreshActiveMessages = useCallback(async () => {
    if (!activeConvId) return;
    try {
      // Use the latest cached seq as cursor only after this conversation has
      // had a full history fetch; /console preview messages are not enough.
      // Read from ref to avoid including messagesByConv in deps (which
      // would rebuild this recovery callback on every state change).
      const hasLoadedHistory = loadedMessageHistoryRef.current.has(activeConvId);
      const sinceSeq = hasLoadedHistory
        ? maxCachedSeq(messagesByConvRef.current, activeConvId)
        : 0;
      const qs = new URLSearchParams({
        principal_id: principal.id,
        ...(sinceSeq > 0 ? { after_seq: String(sinceSeq) } : {}),
        limit: "50",
      });
      const page = await apiFetch<MessagePage>(
        `/v1/conversations/${activeConvId}/message-page?${qs}`,
        sessionToken,
      );
      const newMsgs = page.messages;
      if (sinceSeq === 0) {
        loadedMessageHistoryRef.current.add(activeConvId);
        setMessagePageState((prev) => ({
          ...prev,
          [activeConvId]: { hasMoreBefore: page.has_more, loadingBefore: false },
        }));
        setMessagesByConv((prev) => mergeFetchedMessages(prev, activeConvId, newMsgs));
      }
      if (newMsgs.length > 0) {
        // Clear "thinking" indicator for any agents that replied.
        const repliedSenderIds = thinkingMarkerClearIds(newMsgs);
        if (repliedSenderIds.size > 0) {
          clearThinkingAgentsByIds(repliedSenderIds);
        }
        for (const m of newMsgs) {
          if (isAgent(agents, m.sender_id)) {
            const agentObj = agents.find((a) => a.id === m.sender_id);
            const backendTraceId = messageTraceId(m);
            trace.event("agent_reply", {
              agent_id: m.sender_id,
              agent_name: agentObj?.name,
              conversation_id: activeConvId,
              content_len: m.content?.length ?? 0,
              source: "recovery_fetch",
              backend_trace_id: backendTraceId ?? null,
            });
          }
          // A recovery fetch after a failed send still drives the optional
          // Pixel World view from any messages it discovers.
          if (pixelWorldEnabled && m.sender_id && activeConvId) {
            emitPixelWorldEvent("pixel_world_animation_driven_from_recovery_fetch", {
              agent_id: m.sender_id,
              conversation_id: activeConvId,
              message_id: m.id,
              source: "recovery_fetch",
            });
            usePixelWorldStore.getState().handleMessage(m.sender_id, activeConvId);
            if (m.content) {
              for (const a of agents) {
                if (m.content.toLowerCase().includes(`@${a.name.toLowerCase()}`)) {
                  usePixelWorldStore.getState().handleMention(a.id);
                }
              }
            }
          }
        }
        if (sinceSeq > 0) {
          setMessagesByConv((prev) => appendIncrementalMessages(prev, activeConvId, newMsgs));
        }
      }
    } catch (err) {
      trace.event("recover_messages_error", { convId: activeConvId, error: String(err) });
    }
  }, [activeConvId, principal.id, sessionToken, clearThinkingAgentsByIds, agents, pixelWorldEnabled]);

  const loadOlderMessages = useCallback(async (): Promise<void> => {
    const conversationId = activeConvId;
    if (!conversationId || loadingOlderConversationIdsRef.current.has(conversationId)) return;
    const current = messagesByConvRef.current[conversationId] ?? [];
    const oldestSeq = current.reduce(
      (oldest, message) => message.server_seq < OPTIMISTIC_SERVER_SEQ
        ? Math.min(oldest, message.server_seq)
        : oldest,
      Number.POSITIVE_INFINITY,
    );
    if (!Number.isFinite(oldestSeq)) return;

    loadingOlderConversationIdsRef.current.add(conversationId);
    setMessagePageState((prev) => ({
      ...prev,
      [conversationId]: {
        hasMoreBefore: prev[conversationId]?.hasMoreBefore ?? true,
        loadingBefore: true,
      },
    }));
    try {
      const query = new URLSearchParams({
        principal_id: principal.id,
        before_seq: String(oldestSeq),
        limit: "50",
      });
      const page = await apiFetch<MessagePage>(
        `/v1/conversations/${conversationId}/message-page?${query}`,
        sessionToken,
      );
      setMessagesByConv((prev) => mergeFetchedMessages(prev, conversationId, page.messages));
      setMessagePageState((prev) => ({
        ...prev,
        [conversationId]: { hasMoreBefore: page.has_more, loadingBefore: false },
      }));
    } catch (error) {
      trace.event("load_older_messages_error", { conversationId, error: String(error) });
      setMessagePageState((prev) => ({
        ...prev,
        [conversationId]: {
          hasMoreBefore: prev[conversationId]?.hasMoreBefore ?? true,
          loadingBefore: false,
        },
      }));
      throw error;
    } finally {
      loadingOlderConversationIdsRef.current.delete(conversationId);
    }
  }, [activeConvId, principal.id, sessionToken]);

  const applyBootstrap = useCallback((
    bootstrap: DashboardBootstrap,
    mode: "replace" | "append" | "refresh",
    unreadRequestSeq?: number,
  ) => {
    const snapshotStartedAt = Date.now();
    const items = bootstrap.conversations.items;
    setKnownPrincipals((prev) => mergeKnownPrincipals(prev, bootstrap.principals));
    setConversations((current) => {
      if (mode === "replace") return items.map((item) => item.conversation);
      const byId = new Map(current.map((conversation) => [conversation.id, conversation]));
      for (const item of items) byId.set(item.conversation.id, item.conversation);
      return [...byId.values()];
    });
    setAgents(bootstrap.agents);
    setHostPlugins(bootstrap.plugins);
    replaceCompanies(bootstrap.companies);
    setRuntimeBindings(bootstrap.runtime_bindings);

    flags.applyBootstrap(bootstrap, mode, snapshotStartedAt);

    const unreadRows = items.map((item) => ({
      conversation_id: item.conversation.id,
      unread_count: item.unread_count,
      mention_count: item.mention_count,
      thread_unread_count: item.thread_unread_count,
    }));
    if (mode === "replace" && unreadRequestSeq !== undefined) {
      commitUnreadRows(unreadRows, unreadRequestSeq);
    } else {
      const pageUnreads = applyLocallyViewed(
        buildUnreadMap(unreadRows),
        locallyViewedConversationIdsRef.current,
        unreadCommitGateRef.current.claimedSeq(),
      );
      setUnreads((current) => ({ ...current, ...pageUnreads }));
    }
    setMessagesByConv((current) => mergePreviewIntoMessages(
      current,
      Object.fromEntries(
        items
          .filter((item) => item.last_message)
          .map((item) => [item.conversation.id, [item.last_message!]]),
      ),
    ));
    // A structural refresh only reconciles the bounded first page. Keep the
    // pagination frontier already reached by the user so loaded older rows do
    // not disappear or get fetched again from an earlier cursor.
    if (mode !== "refresh") {
      setBootstrapNextCursor(bootstrap.conversations.next_cursor);
      setBootstrapHasMore(bootstrap.conversations.has_more);
    }
  }, [commitUnreadRows, flags.applyBootstrap, replaceCompanies]);

  const refreshSnapshot = useCallback(async () => {
    try {
      const bootstrap = await apiFetch<DashboardBootstrap>("/v1/bootstrap?limit=100", sessionToken);
      applyBootstrap(bootstrap, "refresh");
    } catch (err) {
      trace.event("refresh_bootstrap_error", { error: String(err) });
    }
  }, [applyBootstrap, sessionToken]);
  refreshBootstrapRef.current = refreshSnapshot;

  const loadMoreConversations = useCallback(async () => {
    if (!bootstrapHasMore || !bootstrapNextCursor || loadingMoreConversations) return;
    setLoadingMoreConversations(true);
    try {
      const query = new URLSearchParams({ limit: "100", after: bootstrapNextCursor });
      const bootstrap = await apiFetch<DashboardBootstrap>(`/v1/bootstrap?${query}`, sessionToken);
      applyBootstrap(bootstrap, "append");
    } catch (error) {
      trace.event("load_more_conversations_error", { error: String(error) });
    } finally {
      setLoadingMoreConversations(false);
    }
  }, [applyBootstrap, bootstrapHasMore, bootstrapNextCursor, loadingMoreConversations, sessionToken]);

  // Hiding also closes the conversation if it is open.
  const handleHideSession = useCallback(
    (conversationId: string) => flags.hide(conversationId, () => {
      if (activeConvId !== conversationId) return;
      setActiveConvId(null);
      setActiveTabId(null);
      setOpenTabs((previous) => previous.filter((tab) => tab.type !== "conv" || tab.convId !== conversationId));
      try { localStorage.removeItem("choruz_active_conv"); } catch { /* ignore */ }
      setReplyTo(null);
      setOpenThreadRootId(null);
    }),
    [activeConvId, flags.hide],
  );

  // ---- Select conversation ----
  const selectConversation = useCallback(
    (convId: string) => {
      const span = trace.start("select_conversation", { convId });
      setActiveConvId(convId);
      setActiveTabId(convId);
      // Add conversation tab if not already open
      setOpenTabs(prev => {
        if (prev.some(t => t.type === 'conv' && t.convId === convId)) return prev;
        return [...prev, { type: 'conv' as const, convId }];
      });
      try { localStorage.setItem("choruz_active_conv", convId); } catch {}
      setShowSidebar(false);
      trackEvent("switch_conversation", { conversation_id: convId });
      // Mark conversation as viewed on server (resets unread counts)
      apiFetch(`/v1/conversations/${convId}/view`, sessionToken, { method: "POST" }).catch(() => {});
      // Clear local unread immediately for responsive UI, and register the
      // view in the locally-viewed overlay so an in-flight unread response
      // already in flight can't resurrect the badge. INVARIANT: every
      // local clear-on-view MUST be paired with registerLocallyViewed —
      // the registry semantics are pinned in lib/messages/thread-unreads.test.ts,
      // but this pairing itself is wiring no unit test reaches; removing
      // it reintroduces the Gate-7 F1 badge-resurrection race. Thread
      // unread is intentionally preserved — it clears per thread, not per
      // conversation.
      registerLocallyViewed(
        locallyViewedConversationIdsRef.current,
        convId,
        unreadCommitGateRef.current.claimedSeq(),
      );
      setUnreads((prev) => clearConversationUnread(prev, convId));
      const conv = conversations.find((c) => c.id === convId);
      const companyId = conv?.workspace_id ?? activeCompanyId;
      if (companyId) {
        activeConversationByCompanyRef.current.set(companyId, convId);
      }
      const agentPeer = conv ? agentPeerId(conv, principal.id, agents) : null;
      if (agentPeer) {
        trace.event("select_conv_terminal_mode", { convId, agentId: agentPeer });
        span.end({ mode: "terminal", agentId: agentPeer });
        return;
      }
      // Incremental fetch: if full history is already loaded for this session,
      // use since_seq; otherwise fetch the first page and merge with previews.
      const hasLoadedHistory = loadedMessageHistoryRef.current.has(convId);
      const sinceSeq = hasLoadedHistory
        ? maxCachedSeq(messagesByConvRef.current, convId)
        : 0;
      const qs = new URLSearchParams({
        principal_id: principal.id,
        ...(sinceSeq > 0 ? { after_seq: String(sinceSeq) } : {}),
        limit: "50",
      });
      const fetchSpan = trace.start("fetch_messages", { convId, sinceSeq });
      apiFetch<MessagePage>(
        `/v1/conversations/${convId}/message-page?${qs}`,
        sessionToken,
      )
        .then((page) => {
          const newMsgs = page.messages;
          fetchSpan.end({ count: newMsgs.length, incremental: sinceSeq > 0 });
          if (sinceSeq > 0) {
            // Append incremental results to existing cache.
            setMessagesByConv((prev) => appendIncrementalMessages(prev, convId, newMsgs));
          } else {
            // First load may race with WebSocket delivery. Merge the full fetched page
            // with anything cached mid-flight so older history is backfilled
            // instead of being filtered out by the incremental path.
            loadedMessageHistoryRef.current.add(convId);
            setMessagePageState((prev) => ({
              ...prev,
              [convId]: { hasMoreBefore: page.has_more, loadingBefore: false },
            }));
            setMessagesByConv((prev) => mergeFetchedMessages(prev, convId, newMsgs));
          }
        })
        .catch((err) => { fetchSpan.end({ error: String(err) }); });
      span.end({ mode: "chat", convType: conv?.conversation_type });
    },
    [principal.id, sessionToken, trackEvent, conversations, activeCompanyId],
  );

  // ---- Send message ----
  const handleSendMessage = useCallback(
    async (content: string) => {
      if (!activeConvId) return;
      // Inherit the click's trace id rather than starting a new one — the
      // global click listener in `startInteractionTracking` already calls
      // `beginTrace()` when the Send button (or Enter key) fires, so a
      // second `beginTrace()` here would fork one physical user action
      // into two unrelated trace chains.
      const sendSpan = trace.start("send_message", {
        conversation_id: activeConvId,
        content_len: content.length,
      });
      trackEvent("send_message", { conversation_id: activeConvId });

      const mentionedAgentIds = mentionedAgentIdsIn(activeConv, agents, content);
      if (mentionedAgentIds.size > 0) markAgentsThinking(mentionedAgentIds);

      const metadata: Record<string, unknown> = replyTo
        ? { reply_to_id: replyTo.id }
        : {};
      const replyTargetId = replyTo?.id;

      const idempotencyKey = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
      const optimisticMsg: ChatMessage = {
        id: idempotencyKey,
        workspace_id: activeConv?.workspace_id ?? "",
        conversation_id: activeConvId,
        sender_id: principal.id,
        content,
        content_type: "text",
        metadata,
        edited_at: null,
        edited_by: null,
        server_seq: OPTIMISTIC_SERVER_SEQ,
        idempotency_key: idempotencyKey,
        created_at: new Date().toISOString(),
      };
      setMessagesByConv((prev) => ({
        ...prev,
        [activeConvId]: [...(prev[activeConvId] ?? []), optimisticMsg],
      }));

      try {
        await apiFetch<ChatMessage>("/v1/messages", sessionToken, {
          method: "POST",
          body: JSON.stringify({
            actor_id: principal.id,
            conversation_id: activeConvId,
            idempotency_key: idempotencyKey,
            content,
            content_type: "text",
            metadata,
          }),
        });
        if (replyTargetId) {
          setReplyTo((current) => current?.id === replyTargetId ? null : current);
        }
        // WebSocket will push confirmation with client_msg_id, replacing the
        // optimistic message. No need for HTTP refresh.
        sendSpan.end({ mentionedAgents: mentionedAgentIds.size });
      } catch (err) {
        sendSpan.end({ error: String(err) });
        setMessagesByConv((prev) => ({
          ...prev,
          [activeConvId]: rollbackOptimisticMessage(
            prev[activeConvId] ?? [],
            idempotencyKey,
          ),
        }));
        throw err;
      } finally {
        // The send action is done from the FE's perspective. Clear the
        // active trace id so it doesn't leak into a later unrelated user
        // action (the agent's asynchronous reply is correlated via the
        // BACKEND trace_id on the WS event metadata, not this FE one).
        //
        // Pass our own trace id so we only clear if it still matches —
        // otherwise, if the user already started a NEW action B while
        // send A was in flight, we'd stomp B's trace on A's completion.
        endTrace(sendSpan.traceId);
      }
    },
    [activeConvId, activeConv, agents, sessionToken, principal.id, trackEvent, replyTo, markAgentsThinking],
  );

  // ---- Thread side panel ----

  // Guards stale fetchThread settlements: opening thread B while A's fetch
  // is still in flight must not let A's .catch/.finally clobber B's
  // loading/error state (or re-anchor B's panel to A's root).
  const openThreadRequestSeqRef = useRef(0);

  const handleOpenThread = useCallback(
    (msgId: string) => {
      if (!activeConvId) return;
      // Canonicalize locally: clicking "open thread" on a broadcast reply
      // must open its ROOT's thread, not start a nested one (threads are
      // flat). Local resolution can still land on a non-canonical id when
      // history is page-limited — the server-side re-anchor below corrects
      // that.
      const byId = new Map(activeMessages.map((m) => [m.id, m]));
      const clicked = byId.get(msgId);
      const rootId = (clicked && resolveThreadRoot(clicked, byId)) || msgId;
      trace.event("open_thread", { rootId, convId: activeConvId });
      const requestSeq = ++openThreadRequestSeqRef.current;
      const isCurrent = () => openThreadRequestSeqRef.current === requestSeq;
      setOpenThreadRootId(rootId);
      setThreadError(null);
      setThreadLoading(true);
      const convId = activeConvId;
      // Authoritative replies (history may be truncated locally) into the
      // shared message store, then re-anchor the panel on the SERVER's
      // canonical root — if local resolution stopped short (unloaded
      // parent), the panel would otherwise point at a reply id whose
      // rollup bucket is empty.
      fetchThread(sessionToken, convId, rootId)
        .then((detail) => {
          // The merge is safe regardless of staleness (idempotent into the
          // shared store); the panel-state writes below are not.
          setMessagesByConv((prev) => ({
            ...prev,
            [convId]: mergeThreadReplies(prev[convId] ?? [], [detail.root, ...detail.replies]),
          }));
          if (!isCurrent()) return;
          setOpenThreadRootId((current) => (current === rootId ? detail.root.id : current));
        })
        .catch((err) => {
          if (!isCurrent()) return;
          setThreadError(err instanceof Error ? err.message : String(err));
        })
        .finally(() => {
          if (!isCurrent()) return;
          setThreadLoading(false);
        });
      // No receipt POST here: the throttle effect below owns the receipt.
      // It fires once when the panel shows its (cached or fetched) replies
      // — posting here too produced a deterministic duplicate
      // markThreadViewed + unreads fetch on every open.
    },
    [activeConvId, activeMessages, sessionToken],
  );

  const handleCloseThread = useCallback(() => {
    setOpenThreadRootId(null);
    setThreadError(null);
  }, []);

  // Close the panel when switching conversations — the root id is scoped to
  // the conversation it came from.
  useEffect(() => {
    setOpenThreadRootId(null);
    setThreadError(null);
  }, [activeConvId]);

  // Keep the thread read receipt current while the panel is open: new
  // replies streaming in over WS are "seen". THROTTLED, not debounced — a
  // trailing debounce that resets per reply would starve indefinitely
  // under a steady sub-window stream; the throttle guarantees at most one
  // POST per window and at least one after the last reply (the server
  // computes MAX(seq) itself, so a pending POST always covers everything
  // up to the moment it fires). A pending receipt is FLUSHED when the
  // panel closes / switches threads / unmounts, so the last batch of
  // replies is never dropped.
  const openThreadReplyCount = openThreadReplies.length;
  const threadViewTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingThreadViewRef = useRef<{ convId: string; rootId: string } | null>(null);
  const postThreadReceipt = useCallback(
    (convId: string, rootId: string) => {
      markThreadViewed(sessionToken, convId, rootId)
        .then(() => refreshUnreadsDebounced())
        .catch(() => {});
    },
    [sessionToken, refreshUnreadsDebounced],
  );
  useEffect(() => {
    if (!openThreadRootId || !activeConvId || openThreadReplyCount === 0) return;
    // Throttle: an already-armed timer covers this reply too (the receipt
    // is content-free; the server snapshots MAX(seq) when it lands).
    if (!threadViewTimerRef.current) {
      const convId = activeConvId;
      const rootId = openThreadRootId;
      pendingThreadViewRef.current = { convId, rootId };
      threadViewTimerRef.current = setTimeout(() => {
        threadViewTimerRef.current = null;
        pendingThreadViewRef.current = null;
        postThreadReceipt(convId, rootId);
      }, THREAD_VIEW_RECEIPT_THROTTLE_MS);
    }
  }, [openThreadRootId, activeConvId, openThreadReplyCount, postThreadReceipt]);
  // Flush the pending receipt when the thread/conversation changes or the
  // component unmounts — replies seen in the final window must still be
  // marked read.
  useEffect(() => {
    return () => {
      if (threadViewTimerRef.current) {
        clearTimeout(threadViewTimerRef.current);
        threadViewTimerRef.current = null;
      }
      const pending = pendingThreadViewRef.current;
      if (pending) {
        pendingThreadViewRef.current = null;
        postThreadReceipt(pending.convId, pending.rootId);
      }
    };
  }, [openThreadRootId, activeConvId, postThreadReceipt]);

  const handleSendThreadReply = useCallback(
    async (content: string, broadcast: boolean) => {
      if (!activeConvId || !openThreadRootId) return;
      const sendSpan = trace.start("send_thread_reply", {
        conversation_id: activeConvId,
        thread_root: openThreadRootId,
        broadcast,
        content_len: content.length,
      });
      trackEvent("send_thread_reply", { conversation_id: activeConvId, broadcast });

      // Same mention scan as handleSendMessage, @all included: the backend
      // router wakes agents on @all regardless of thread context.
      const mentionedAgentIds = mentionedAgentIdsIn(activeConv, agents, content);
      if (mentionedAgentIds.size > 0) markAgentsThinking(mentionedAgentIds);

      const metadata: Record<string, unknown> = {
        reply_to_id: openThreadRootId,
        thread: true,
        broadcast,
      };
      const idempotencyKey = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
      const optimisticMsg: ChatMessage = {
        id: idempotencyKey,
        workspace_id: activeConv?.workspace_id ?? "",
        conversation_id: activeConvId,
        sender_id: principal.id,
        content,
        content_type: "text",
        metadata,
        edited_at: null,
        edited_by: null,
        server_seq: OPTIMISTIC_SERVER_SEQ,
        idempotency_key: idempotencyKey,
        created_at: new Date().toISOString(),
      };
      setMessagesByConv((prev) => ({
        ...prev,
        [activeConvId]: [...(prev[activeConvId] ?? []), optimisticMsg],
      }));

      try {
        await apiFetch<ChatMessage>("/v1/messages", sessionToken, {
          method: "POST",
          body: JSON.stringify({
            actor_id: principal.id,
            conversation_id: activeConvId,
            idempotency_key: idempotencyKey,
            content,
            content_type: "text",
            metadata,
          }),
        });
        sendSpan.end({ mentionedAgents: mentionedAgentIds.size });
      } catch (err) {
        sendSpan.end({ error: String(err) });
        // Roll the optimistic reply back so the composer error is honest.
        // rollbackOptimisticMessage matches the sentinel seq too — see its
        // doc (and unit test) for the WS-raced-ahead data-loss case.
        setMessagesByConv((prev) => ({
          ...prev,
          [activeConvId]: rollbackOptimisticMessage(prev[activeConvId] ?? [], idempotencyKey),
        }));
        throw err;
      } finally {
        endTrace(sendSpan.traceId);
      }
    },
    [activeConvId, openThreadRootId, activeConv, agents, sessionToken, principal.id, trackEvent, markAgentsThinking],
  );

  const handleUploadAttachment = useCallback(
    async (file: File) => {
      if (!activeConvId || !activeConv) return;
      const convId = activeConvId;
      const workspaceId = activeConv.workspace_id ?? "";
      if (file.size > MAX_ATTACHMENT_BYTES) {
        throw new Error("Attachments are currently limited to 5 MB.");
      }
      const uploadSpan = trace.start("upload_attachment", {
        conversation_id: convId,
        filename: file.name,
        size_bytes: file.size,
      });
      let attachmentId: string | null = null;
      let optimisticIdempotencyKey: string | null = null;
      try {
        const arrayBuffer = await file.arrayBuffer();
        const bytes = new Uint8Array(arrayBuffer);
        // Convert bytes to base64 in chunks to avoid stack blowups.
        let binary = "";
        const chunkSize = 0x8000;
        for (let i = 0; i < bytes.length; i += chunkSize) {
          const chunk = bytes.subarray(i, i + chunkSize);
          binary += String.fromCharCode(...chunk);
        }
        const dataBase64 = btoa(binary);

        const attachment = await apiFetch<{
          id: string;
          filename: string;
          content_type: string;
          size_bytes: number;
          download_path: string;
        }>("/v1/attachments", sessionToken, {
          method: "POST",
          body: JSON.stringify({
            actor_id: principal.id,
            filename: file.name,
            content_type: file.type || "application/octet-stream",
            data_base64: dataBase64,
          }),
        });
        attachmentId = attachment.id;

        const idempotencyKey = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
        optimisticIdempotencyKey = idempotencyKey;
        const attachmentMetadata = {
          attachment_id: attachment.id,
          filename: attachment.filename,
          mime_type: attachment.content_type,
          content_type: attachment.content_type,
          size_bytes: attachment.size_bytes,
          download_path: attachment.download_path,
        };
        const optimisticMsg: ChatMessage = {
          id: idempotencyKey,
          workspace_id: workspaceId,
          conversation_id: convId,
          sender_id: principal.id,
          content: `Attachment: ${attachment.filename}`,
          content_type: "attachment",
          metadata: attachmentMetadata,
          edited_at: null,
          edited_by: null,
          server_seq: OPTIMISTIC_SERVER_SEQ,
          idempotency_key: idempotencyKey,
          created_at: new Date().toISOString(),
        };
        setMessagesByConv((prev) => ({
          ...prev,
          [convId]: [...(prev[convId] ?? []), optimisticMsg],
        }));

        await apiFetch<ChatMessage>("/v1/messages", sessionToken, {
          method: "POST",
          body: JSON.stringify({
            actor_id: principal.id,
            conversation_id: convId,
            idempotency_key: idempotencyKey,
            content: `Attachment: ${attachment.filename}`,
            content_type: "attachment",
            metadata: attachmentMetadata,
          }),
        });

        if (activeConvId === convId) {
          void refreshActiveMessages();
        }
        uploadSpan.end({ ok: true, attachment_id: attachment.id });
      } catch (err) {
        if (optimisticIdempotencyKey) {
          setMessagesByConv((prev) => ({
            ...prev,
            [convId]: (prev[convId] ?? []).filter(
              (msg) => msg.idempotency_key !== optimisticIdempotencyKey,
            ),
          }));
        }
        let detail = err instanceof Error ? err.message : "Upload failed";
        if (attachmentId) {
          try {
            await apiFetch<void>(
              `/v1/attachments/${attachmentId}?actor_id=${principal.id}`,
              sessionToken,
              { method: "DELETE" },
            );
          } catch (cleanupErr) {
            detail = cleanupErr instanceof Error
              ? `Attachment uploaded but could not be shared or rolled back: ${cleanupErr.message}`
              : "Attachment uploaded but could not be shared or rolled back.";
          }
        }
        uploadSpan.end({ error: detail });
        throw new Error(detail);
      }
    },
    [
      activeConvId,
      activeConv,
      sessionToken,
      principal.id,
      refreshActiveMessages,
    ],
  );

  const handleComposerSend = useCallback(async (content: string, attachments: File[]) => {
    for (const file of attachments) {
      await handleUploadAttachment(file);
    }
    if (content) {
      await handleSendMessage(content);
    }
  }, [handleSendMessage, handleUploadAttachment]);

  // ---- Open direct conversation on avatar click ----
  const handleAvatarClick = useCallback(
    async (principalId: string) => {
      // Inherit the click's trace (global click tracker already called
      // beginTrace on the avatar's DOM click).
      const span = trace.start("avatar_click", { principalId, isAgent: isAgent(agents, principalId) });
      if (principalId === principal.id) { span.end({ skipped: "self" }); return; }
      const existing = conversations.find(
        (c) =>
          c.conversation_type === "direct" &&
          c.members[principalId] &&
          c.members[principal.id],
      );
      if (existing) {
        span.end({ found: "existing", convId: existing.id });
        selectConversation(existing.id);
        return;
      }
      try {
        const createSpan = trace.start("create_direct_conv", { principalId });
        const conv = await apiFetch<Conversation>(
          "/v1/conversations/direct",
          sessionToken,
          {
            method: "POST",
            body: JSON.stringify({
              actor_id: principal.id,
              peer_principal_id: principalId,
              workspace_id: activeConv?.workspace_id ?? activeCompanyId ?? undefined,
            }),
          },
        );
        createSpan.end({ convId: conv.id });
        await refreshSnapshot();
        selectConversation(conv.id);
        span.end({ found: "created", convId: conv.id });
      } catch (err) {
        span.end({ error: String(err) });
      }
    },
    [principal.id, conversations, selectConversation, sessionToken, activeConv, activeCompanyId, refreshSnapshot, agents],
  );

  // ---- Terminal mode detection ----
  // The agent on the other side of the active DM, or null when the active
  // conversation is not an agent DM.
  const terminalAgentId = useMemo(
    () => (activeConv ? agentPeerId(activeConv, principal.id, agents) : null),
    [activeConv, principal.id, agents],
  );
  const isAgentDm = terminalAgentId !== null;

  const terminalAgentName = useMemo(
    () => (terminalAgentId ? principalName(principal, agents, terminalAgentId) : null),
    [terminalAgentId, principal, agents],
  );

  const terminalBindingId = useMemo(
    () =>
      activeConv && terminalAgentId
        ? findTerminalBinding(runtimeBindings, activeConv.id, terminalAgentId)?.id ?? null
        : null,
    [activeConv, terminalAgentId, runtimeBindings],
  );
  // An agent created through the API alone has no runtime binding and talks
  // over messages, so the message view is right for it; the trace tells that
  // case apart from a binding the feed has not delivered yet.
  const agentDmHasBinding = useMemo(
    () =>
      activeConv && terminalAgentId
        ? findAgentDmBinding(runtimeBindings, activeConv.id, terminalAgentId) !== undefined
        : true,
    [activeConv, terminalAgentId, runtimeBindings],
  );
  useEffect(() => {
    if (!activeConv || !terminalAgentId || agentDmHasBinding) return;
    trace.event("terminal_binding_missing", {
      convId: activeConv.id,
      agentId: terminalAgentId,
      bindings: runtimeBindings.length,
    });
  }, [activeConv, terminalAgentId, agentDmHasBinding, runtimeBindings.length]);

  // Terminal bindings for every open conv tab — lets us keep each TerminalView
  // mounted (with display toggled) so switching tabs doesn't tear down the
  // WebSocket and kill the backend PTY.  Only closing the tab unmounts.
  const openTerminalBindings = useMemo(
    () =>
      terminalBindingsForOpenTabs(
        openTabs.flatMap((tab) => (tab.type === "conv" ? [tab.convId] : [])),
        conversations,
        agents,
        principal.id,
        runtimeBindings,
      ),
    [openTabs, conversations, runtimeBindings, agents, principal.id],
  );

  // ---- Message search ----
  // The detail-panel Search tab scopes to the active conversation by passing
  // `conversation_id`; without it, the gateway falls back to searching every
  // conversation the user belongs to (used by any future global search UI).
  const search = useMessageSearch({
    principalId: principal.id,
    sessionToken,
    activeConversationId: activeConvId,
    onSelectResult: selectConversation,
  });

  // ---- Conversation header info ----
  const chatTitle = activeConv
    ? conversationDisplayName(activeConv, principal, agents)
    : "Choruz";
  // Agent DM detection drives the header's amber "talking to an agent"
  // visual mode (see .chat-header.is-agent in app/styles/chat-header.css). Not every
  // agent conversation is a DM — groups can contain agents — but DMs
  // are where the visual shift reads clearly.
  const agentDmPlacement = useMemo(() => {
    if (!terminalAgentId || !activeConv) return null;
    const binding = findTerminalBinding(runtimeBindings, activeConv.id, terminalAgentId);
    const machine = bindingMachineLabel(binding, runtimeHosts);
    return binding?.harness_account_name ? `${machine} · ${binding.harness_account_name}` : machine;
  }, [activeConv, terminalAgentId, runtimeBindings, runtimeHosts]);
  const agentAccountNames = useMemo(() => new Map(
    runtimeBindings
      .filter((binding) => binding.state !== "disabled" && binding.harness_account_name)
      .map((binding) => [binding.agent_principal_id, binding.harness_account_name!] as const),
  ), [runtimeBindings]);
  const chatSubtitle = activeConv
    ? (isAgentDm
        ? `AI agent${agentDmPlacement ? ` · ${agentDmPlacement}` : ""}`
        : `${Object.keys(activeConv.members).length} members`)
    : "Select a conversation";

  // ---- Swipe gesture for mobile sidebar ----
  const shellRef = useRef<HTMLElement>(null);
  useEdgeSwipe({ ref: shellRef, setOpen: setShowSidebar });

  // ---- Render ----
  return (
    <main className="chat-shell" ref={shellRef}>
      {/* Mobile backdrop */}
      {(showSidebar || showDetail) && (
        <div
          className="mobile-backdrop"
          onClick={() => {
            setShowSidebar(false);
            setShowDetail(false);
          }}
        />
      )}

      {/* ---- Sidebar ---- */}
      <Sidebar
        open={showSidebar}
        principal={principal}
        conversations={companyConversations}
        agents={companyAgents}
        messagesByConv={messagesByConv}
        pinnedConversations={sidebarPinnedConversations}
        archivedConversations={sidebarArchivedConversations}
        hiddenConversations={flags.hidden}
        runtimeBindings={runtimeBindings}
        pinPendingConversationIds={flags.pendingPinIds}
        archivePendingConversationIds={flags.pendingArchiveIds}
        hidePendingConversationIds={flags.pendingHiddenIds}
        activeConvId={activeConvId}
        onTogglePin={flags.togglePin}
        onToggleArchive={flags.toggleArchive}
        onRestoreHiddenSession={flags.restore}
        onHideSession={handleHideSession}
        onSelectConversation={selectConversation}
        onCreateAgent={() => setShowCreateAgent(true)}
        onManageHarnessAccounts={() => setShowHarnessAccounts(true)}
        onCreateGroup={() => setShowCreateGroup(true)}
        sessionToken={sessionToken}
        refreshSnapshot={refreshSnapshot}
        hasMoreConversations={bootstrapHasMore}
        loadingMoreConversations={loadingMoreConversations}
        onLoadMoreConversations={() => { void loadMoreConversations(); }}
        trackEvent={trackEvent}
        companies={companies}
        activeCompanyId={activeCompanyId}
        onSelectCompany={selectCompany}
        onCreateCompany={() => setShowCreateCompany(true)}
        onToggleAgentsActive={toggleAgentsActive}
        pluginActions={[
          ...(remoteSshEnabled ? [{
            ...remoteSshSidebarAction,
            onSelect: () => { trace.event("open_servers"); setShowServers(true); },
          }] : []),
          ...(remoteControlEnabled ? [{
            ...remoteControlSidebarAction,
            onSelect: () => { trace.event("open_remote_control"); setShowRemoteControl(true); },
          }, {
            id: "import-workspace-sessions",
            label: "Import Sessions",
            onSelect: () => {
              trace.event("open_workspace_session_import");
              setShowWorkspaceSessionImport(true);
            },
          }] : []),
          ...(pixelWorldEnabled ? [{
            ...pixelWorldSidebarAction,
            onSelect: () => {
              emitPixelWorldEvent("pixel_world_menu_clicked", { source: "sidebar_plus_menu" });
              setShowPixelWorld((open) => {
                const next = !open;
                emitPixelWorldEvent(next ? "pixel_world_opened" : "pixel_world_closed", {
                  via: "sidebar_menu",
                });
                return next;
              });
            },
          }] : []),
        ]}
        onArchiveCompany={archiveCompany}
        onUnarchiveCompany={unarchiveCompany}
        onDeleteCompany={handleDeleteCompany}
        onRenameCompany={renameCompany}
        onChangeCompanyWorkspace={changeCompanyWorkspace}
        onOpenCompanyMachines={remoteControlEnabled ? setMachinesCompanyId : undefined}
        unreads={unreads}
        style={{ "--sidebar-width": `${sidebarResize.width}px` } as CSSProperties}
        extraClassName={sidebarResize.resizing ? "resizing" : undefined}
        onOpenFile={openFile}
      />

      {/* ---- Sidebar resize handle ---- */}
      <ResizeHandle resize={sidebarResize} label="Resize sidebar" />

      {/* ---- Chat main area ---- */}
      <div className="chat-main">
        {/* Tab bar — shown when 2+ tabs open */}
        {openTabs.length > 1 && (
          <div className="editor-tab-bar">
            {openTabs.map(tab => {
              const id = tab.type === 'conv' ? tab.convId : tab.path;
              const isActive = activeTabId === id;
              const label = tab.type === 'conv'
                ? (() => { const c = conversations.find(c => c.id === tab.convId); return c ? conversationDisplayName(c, principal, agents) : 'Chat'; })()
                : tab.path.split('/').pop() || 'File';
              const isConv = tab.type === 'conv';
              return (
                <div key={id} className={`editor-tab${isActive ? ' active' : ''}`}>
                  <button
                    type="button"
                    className="editor-tab-main"
                    onClick={() => {
                      trace.event("switch_tab", { tabId: id, tabType: isConv ? "conv" : "file", label });
                      setActiveTabId(id);
                      if (isConv) setActiveConvId(tab.convId);
                    }}
                  >
                    <span className="editor-tab-icon" aria-hidden="true">
                      {isConv
                        ? <Hash size={12} strokeWidth={2} />
                        : <FileCode2 size={12} strokeWidth={1.75} />}
                    </span>
                    <span className="editor-tab-name">{label}</span>
                    {tab.type === 'file' && tab.dirty && <span className="editor-tab-dot" />}
                  </button>
                  <button
                    type="button"
                    className="editor-tab-close"
                    aria-label={`Close ${label}`}
                    onClick={() => { trace.event("close_tab", { tabId: id, tabType: isConv ? "conv" : "file", label }); closeTab(id); }}
                  >
                    <X size={12} aria-hidden="true" />
                  </button>
                </div>
              );
            })}
          </div>
        )}

        {/* Show chat or file editor based on active tab */}
        {/* Show chat when active tab is a conversation (or no tab selected), file editor when it's a file */}
        {!openTabs.some(t => t.type === 'file' && tabId(t) === activeTabId) ? (
          <>
            <ChatHeader
              activeConv={activeConv}
              chatTitle={chatTitle}
              chatSubtitle={chatSubtitle}
              isAgentDm={isAgentDm}
              showDetail={showDetail}
              wsStatus={wsStatus}
              onToggleSidebar={() => { trace.event("toggle_sidebar", { open: !showSidebar }); setShowSidebar(!showSidebar); }}
              onToggleDetail={() => { trace.event("toggle_detail", { open: !showDetail }); setShowDetail(!showDetail); }}
            />
            <ChannelConversationTabs
              pluginTabs={showChannelTasksTab ? [kanbanConversationTab] : []}
              activeView={activeConversationView}
              onSelectView={(view) => {
                trace.event("conversation_view_switch", { view, convId: activeConvId });
                setActiveConversationView(view);
              }}
            />

            {/* Persistent terminal views — mount one per open terminal tab
                so switching tabs doesn't tear down the WebSocket (and thus
                doesn't kill the backend PTY).  Only closing the tab removes
                the entry from openTerminalBindings and unmounts. */}
            {openTerminalBindings.map(({ convId, bindingId }) => (
              <div
                key={bindingId}
                className={
                  convId === activeConvId && activeConversationView === "chat"
                    ? "terminal-pane"
                    : "terminal-pane is-hidden"
                }
              >
                <Suspense fallback={null}>
                  <TerminalView bindingId={bindingId} sessionToken={sessionToken} gatewayBaseUrl={gatewayBaseUrl} />
                </Suspense>
              </div>
            ))}

            {activeConv ? (
              <>
                {showChannelTasksTab && activeConversationView === "tasks" ? (
                  <KanbanBoard
                    tasks={activeConvId ? channelTasksByConv[activeConvId] ?? [] : []}
                    visibleAssignees={visibleChannelTaskAssignees}
                    loading={Boolean(activeConvId && loadingChannelTaskConvIds.has(activeConvId))}
                    error={activeConvId ? channelTaskLoadErrors[activeConvId] ?? null : null}
                    mutatingTaskIds={mutatingChannelTaskIds}
                    onPatchTask={handlePatchChannelTask}
                  />
                ) : terminalBindingId !== null ? null : (
                  /* A live terminal pane replaces the message list + composer. */
                  <div
                    id="conversation-chat-panel"
                    role="tabpanel"
                    aria-labelledby="conversation-chat-tab"
                    className="chat-with-thread"
                  >
                    <div className="chat-primary">
                      <MessageList
                        messages={timelineMessages}
                        principal={principal}
                        agents={agents}
                        activeConv={activeConv}
                        isTerminalChat={isAgentDm}
                        thinkingAgents={thinkingAgents}
                        agentAccountNames={agentAccountNames}
                        onAvatarClick={activeConv.conversation_type === "group" ? handleAvatarClick : undefined}
                        onReply={(reply) => { trace.event("reply_message", { msgId: reply.id, senderName: reply.senderName, convId: activeConvId }); setReplyTo(reply); }}
                        onCreateTaskFromMessage={canCreateChannelTaskFromActiveMessage ? handleOpenCreateTaskFromMessage : undefined}
                        threadRollups={threadRollups}
                        quotedMessages={quotedMessages}
                        onOpenThread={handleOpenThread}
                        initialActionsOpen={initialMessageActionsOpen}
                        hasOlderMessages={messagePageState[activeConv.id]?.hasMoreBefore ?? false}
                        loadingOlderMessages={messagePageState[activeConv.id]?.loadingBefore ?? false}
                        onLoadOlderMessages={loadOlderMessages}
                      />

                      <ChatInput
                        principal={principal}
                        agents={agents}
                        activeConv={activeConv}
                        placeholder={
                          isAgentDm
                            ? `Send to ${terminalAgentName ?? chatTitle} terminal...`
                            : `Message ${chatTitle}...`
                        }
                        onSendMessage={handleComposerSend}
                        replyTo={replyTo}
                        onCancelReply={() => { trace.event("cancel_reply", { convId: activeConvId }); setReplyTo(null); }}
                      />
                    </div>
                    {openThreadRoot && (
                      <ThreadPanel
                        root={openThreadRoot}
                        replies={openThreadReplies}
                        principal={principal}
                        agents={agents}
                        activeConv={activeConv}
                        loading={threadLoading}
                        error={threadError}
                        onClose={handleCloseThread}
                        onSendReply={handleSendThreadReply}
                      />
                    )}
                  </div>
                )}
              </>
            ) : (
              <EmptyState
                icon={<MessagesSquare size={40} strokeWidth={1.25} />}
                title="Welcome to Choruz"
                description="Select a conversation from the sidebar or create a new group to get started."
              />
            )}
          </>
        ) : activeTabId ? (
          <FileEditor
            filePath={activeTabId}
            workspaceId={activeFileTab?.type === "file" ? activeFileTab.workspaceId : activeCompanyId}
            sessionToken={sessionToken}
            onClose={() => closeTab(activeTabId!)}
            onDirty={(dirty) => setOpenTabs(prev => prev.map(t => t.type === 'file' && t.path === activeTabId ? { ...t, dirty } : t))}
          />
        ) : null}
      </div>

      <ChatModals
        showPixelWorld={pixelWorldEnabled && showPixelWorld}
        pixelConversations={companyConversations}
        pixelAgents={companyAgents}
        messagesByConv={messagesByConv}
        activeConvId={activeConvId}
        onSelectConversation={selectConversation}
        onClosePixelWorld={() => setShowPixelWorld(false)}
        showDetail={showDetail}
        activeConv={activeConv}
        principal={principal}
        agents={agents}
        sessionToken={sessionToken}
        runtimeBindings={runtimeBindings}
        runtimeHosts={runtimeHosts}
        workspaceGitEnabled={workspaceGitEnabled}
        agentSkillsEnabled={agentSkillsEnabled}
        mathcodeEnabled={mathcodeEnabled}
        multiHarnessAccounts={multiHarnessAccounts}
        onMultiHarnessAccountsChange={async (enabled) => {
          if (activeCompanyId) await setMultiHarnessAccounts(activeCompanyId, enabled);
        }}
        onCloseDetail={() => setShowDetail(false)}
        detailResize={detailResize}
        refreshSnapshot={refreshSnapshot}
        searchQuery={search.query}
        searchResults={search.results}
        searchLoading={search.loading}
        onSearchInput={search.handleInput}
        onSearchResultClick={search.handleResultClick}
        showCreateGroup={showCreateGroup}
        activeCompanyId={activeCompanyId}
        onCloseCreateGroup={() => setShowCreateGroup(false)}
        onCreatedGroup={(convId) => {
          setShowCreateGroup(false);
          trackEvent("create_group", { conversation_id: convId });
          selectConversation(convId);
        }}
        showCreateAgent={showCreateAgent}
        onCloseCreateAgent={() => setShowCreateAgent(false)}
        onCreatedAgent={(convId) => {
          setShowCreateAgent(false);
          trackEvent("create_agent", { conversation_id: convId });
          selectConversation(convId);
        }}
        showHarnessAccounts={showHarnessAccounts}
        onCloseHarnessAccounts={() => setShowHarnessAccounts(false)}
        showCreateCompany={showCreateCompany}
        onCloseCreateCompany={() => setShowCreateCompany(false)}
        onCreatedCompany={async (company) => {
          setShowCreateCompany(false);
          addCompany(company);
          selectCompany(company.id);
          await refreshSnapshot();
        }}
      />

      {createTaskDraft ? (
        <ChannelTaskCreateModal
          message={createTaskDraft.message}
          visibleAssignees={visibleChannelTaskAssignees}
          submitting={creatingTaskFromMessage}
          error={createTaskError}
          onClose={() => {
            if (!creatingTaskFromMessage) {
              setCreateTaskDraft(null);
              setCreateTaskError(null);
            }
          }}
          onSubmit={handleSubmitCreateTaskFromMessage}
        />
      ) : null}

      {/* ---- Server Manager Modal ---- */}
      {remoteSshEnabled && showServers && (
        <RemoteSshModal
          sessionToken={sessionToken}
          onClose={() => setShowServers(false)}
        />
      )}
      {remoteControlEnabled && showRemoteControl && (
        <RemoteControlModal
          sessionToken={sessionToken}
          onClose={() => setShowRemoteControl(false)}
        />
      )}
      {remoteControlEnabled && machinesCompanyId && (() => {
        const company = companies.find((item) => item.id === machinesCompanyId);
        return company ? (
          <RuntimeHostsModal
            sessionToken={sessionToken}
            companyId={company.id}
            companyName={company.name}
            runtimeBindings={runtimeBindings}
            onHostsChanged={company.id === activeCompanyId ? setRuntimeHosts : undefined}
            onClose={() => setMachinesCompanyId(null)}
          />
        ) : null;
      })()}
      {remoteControlEnabled && showWorkspaceSessionImport && (
        <ImportWorkspaceSessionsModal
          sessionToken={sessionToken}
          activeCompanyId={activeCompanyId}
          onClose={closeWorkspaceSessionImport}
          onImported={async (sessions) => {
            closeWorkspaceSessionImport();
            await refreshSnapshot();
            const firstConversation = sessions[0]?.conversation_id;
            if (firstConversation) selectConversation(firstConversation);
          }}
        />
      )}
    </main>
  );
}

export function optimisticChannelTask(
  task: ChannelTask,
  patch: PatchChannelTaskRequest,
  visibleAssignees: Principal[],
): ChannelTask {
  const next: ChannelTask = { ...task };
  if (patch.status) {
    next.status = patch.status;
  }
  if (patch.blocked_reason === null) {
    next.blocked_reason = undefined;
  } else if (patch.blocked_reason !== undefined) {
    next.blocked_reason = patch.blocked_reason;
  }
  if (patch.context_label === null) {
    next.context_label = undefined;
  } else if (patch.context_label !== undefined) {
    next.context_label = patch.context_label;
  }
  next.updated_at = new Date().toISOString();
  if (patch.assignee_principal_id) {
    next.assignee_principal_id = patch.assignee_principal_id;
    const assignee = visibleAssignees.find((candidate) => candidate.id === patch.assignee_principal_id);
    next.assignee_name = assignee?.name ?? task.assignee_name;
    next.assignee_type = assignee?.principal_type ?? task.assignee_type;
  }
  return next;
}

function mergeKnownPrincipals(existing: Principal[], incoming: Principal[]): Principal[] {
  return dedupePrincipals([...existing, ...incoming]).sort((left, right) => left.name.localeCompare(right.name));
}
