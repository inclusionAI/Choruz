import type {
  ChatMessage,
  Conversation,
  Principal,
  RuntimeBindingInfo,
  PinnedConversation,
  ArchivedConversation,
  HiddenConversation,
} from "../api/choruz-types";
import { stripAnsi } from "../terminal/ansi";
import { conversationDisplayName, directPeerId, principalName } from "../api/principals";
import { bindingUsesTerminalTranscript } from "../terminal/terminal-bindings";

export type { PinnedConversation, ArchivedConversation, HiddenConversation } from "../api/choruz-types";

export type SidebarConversationSectionId = "pinned" | "direct" | "group" | "archived";

export type SidebarConversationItem = {
  id: string;
  conversation: Conversation;
  displayName: string;
  sectionId: SidebarConversationSectionId;
  isPinned: boolean;
  pinnedAt: string | null;
  isArchived: boolean;
  archivedAt: string | null;
  latestActivityAt: string;
  isActive: boolean;
  isTerminalDirectMessage: boolean;
};

export type SidebarConversationSection = {
  id: SidebarConversationSectionId;
  title: "Pinned Chats" | "Direct Messages" | "Group Conversations" | "Archived";
  conversations: SidebarConversationItem[];
  conversationIds: string[];
  defaultExpanded: boolean;
  forceExpandedByActive: boolean;
  forceExpandedBySearch: boolean;
  shouldRender: boolean;
};

export type SidebarConversationSectionsResult = {
  sections: SidebarConversationSection[];
  pinned: SidebarConversationSection;
  direct: SidebarConversationSection;
  group: SidebarConversationSection;
  archived: SidebarConversationSection;
  allFilteredConversationIds: string[];
  conversationSectionById: Record<string, SidebarConversationSectionId>;
  activeSectionId: SidebarConversationSectionId | null;
  hasSearchQuery: boolean;
};

export type BuildSidebarConversationSectionsInput = {
  conversations: Conversation[];
  agents: Principal[];
  principal: Principal;
  messagesByConv: Record<string, ChatMessage[]>;
  searchQuery?: string | null;
  pinnedConversations?: PinnedConversation[];
  archivedConversations?: ArchivedConversation[];
  hiddenConversations?: HiddenConversation[];
  activeConvId?: string | null;
  runtimeBindings?: RuntimeBindingInfo[];
};

export function conversationPreviewText({
  messages,
  principal,
  agents,
  hideEmptyPlaceholder = false,
}: {
  messages: ChatMessage[];
  principal: Principal;
  agents: Principal[];
  hideEmptyPlaceholder?: boolean;
}): string | null {
  const lastMsg = messages[messages.length - 1];
  if (!lastMsg) return hideEmptyPlaceholder ? null : "No messages yet";
  const rawPreview =
    lastMsg.content_type === "system"
      ? lastMsg.content
      : lastMsg.content_type === "runtime_transcript"
        ? stripAnsi(lastMsg.content).replace(/\s+/g, " ").trim()
        : `${principalName(principal, agents, lastMsg.sender_id)}: ${lastMsg.content}`;
  return rawPreview || "No messages yet";
}

export function buildSidebarConversationSections({
  conversations,
  agents,
  principal,
  messagesByConv,
  searchQuery,
  pinnedConversations = [],
  archivedConversations = [],
  hiddenConversations = [],
  activeConvId = null,
  runtimeBindings = [],
}: BuildSidebarConversationSectionsInput): SidebarConversationSectionsResult {
  const disabledAgentIds = new Set(
    agents.filter((agent) => agent.disabled).map((agent) => agent.id),
  );
  const pinnedByConversationId = new Map(
    pinnedConversations.map((pin) => [pin.conversation_id, pin.pinned_at]),
  );
  const archivedByConversationId = new Map(
    archivedConversations.map((archive) => [archive.conversation_id, archive.archived_at]),
  );
  const hiddenByConversationId = new Map(
    hiddenConversations.map((hidden) => [hidden.conversation_id, hidden.hidden_at]),
  );
  const normalizedSearch = searchQuery?.trim().toLowerCase() ?? "";
  const hasSearchQuery = normalizedSearch.length > 0;

  const visibleItems = conversations
    .filter((conversation) =>
      isConversationVisible(conversation, principal, disabledAgentIds),
    )
    .map((conversation): SidebarConversationItem => {
      const pinnedAt = pinnedByConversationId.get(conversation.id) ?? null;
      const archivedAt = archivedByConversationId.get(conversation.id) ?? null;
      const displayName = conversationDisplayName(conversation, principal, agents);
      return {
        id: conversation.id,
        conversation,
        displayName,
        sectionId: sectionIdForConversation(conversation, pinnedAt, archivedAt),
        isPinned: pinnedAt !== null,
        pinnedAt,
        isArchived: archivedAt !== null,
        archivedAt,
        latestActivityAt: latestActivityAt(conversation, messagesByConv),
        isActive: conversation.id === activeConvId,
        isTerminalDirectMessage: isTerminalDirectMessage(
          conversation,
          principal,
          runtimeBindings,
        ),
      };
    })
    .filter((item) => !hiddenByConversationId.has(item.id))
    .filter((item) =>
      hasSearchQuery ? item.displayName.toLowerCase().includes(normalizedSearch) : true,
    );

  const pinnedItems = visibleItems
    .filter((item) => item.sectionId === "pinned")
    .sort((a, b) => compareIsoDesc(a.pinnedAt, b.pinnedAt));
  const directItems = visibleItems
    .filter((item) => item.sectionId === "direct")
    .sort(compareLatestActivityDesc);
  const groupItems = visibleItems
    .filter((item) => item.sectionId === "group")
    .sort(compareLatestActivityDesc);
  const archivedItems = visibleItems
    .filter((item) => item.sectionId === "archived")
    .sort((a, b) => compareIsoDesc(a.archivedAt, b.archivedAt));

  const pinned = buildSection("pinned", "Pinned Chats", pinnedItems, {
    defaultExpanded: pinnedItems.length > 0,
    hideWhenEmpty: true,
    hasSearchQuery,
  });
  const direct = buildSection("direct", "Direct Messages", directItems, {
    defaultExpanded: false,
    hideWhenEmpty: false,
    hasSearchQuery,
  });
  const group = buildSection("group", "Group Conversations", groupItems, {
    defaultExpanded: false,
    hideWhenEmpty: false,
    hasSearchQuery,
  });
  const archived = buildSection("archived", "Archived", archivedItems, {
    defaultExpanded: false,
    hideWhenEmpty: true,
    hasSearchQuery,
  });
  const sections = [pinned, direct, group, archived];
  const allFilteredConversationIds = sections.flatMap(
    (section) => section.conversationIds,
  );
  const conversationSectionById: Record<string, SidebarConversationSectionId> = {};
  let activeSectionId: SidebarConversationSectionId | null = null;

  for (const section of sections) {
    for (const item of section.conversations) {
      conversationSectionById[item.id] = section.id;
      if (item.isActive) activeSectionId = section.id;
    }
  }

  return {
    sections,
    pinned,
    direct,
    group,
    archived,
    allFilteredConversationIds,
    conversationSectionById,
    activeSectionId,
    hasSearchQuery,
  };
}

function isConversationVisible(
  conversation: Conversation,
  principal: Principal,
  disabledAgentIds: Set<string>,
): boolean {
  if (conversation.conversation_type !== "direct") return true;
  const otherMember = directPeerId(conversation, principal.id);
  return !(otherMember && disabledAgentIds.has(otherMember));
}

function sectionIdForConversation(
  conversation: Conversation,
  pinnedAt: string | null,
  archivedAt: string | null,
): SidebarConversationSectionId {
  if (archivedAt) return "archived";
  if (pinnedAt) return "pinned";
  return conversation.conversation_type === "direct" ? "direct" : "group";
}

function isTerminalDirectMessage(
  conversation: Conversation,
  principal: Principal,
  runtimeBindings: RuntimeBindingInfo[],
): boolean {
  if (conversation.conversation_type !== "direct") return false;
  const otherMemberId = directPeerId(conversation, principal.id);
  if (!otherMemberId) return false;
  return runtimeBindings.some(
    (binding) =>
      binding.conversation_id === conversation.id &&
      binding.agent_principal_id === otherMemberId &&
      bindingUsesTerminalTranscript(binding) &&
      (!binding.conversation_type || binding.conversation_type === "direct"),
  );
}

function latestActivityAt(
  conversation: Conversation,
  messagesByConv: Record<string, ChatMessage[]>,
): string {
  const messages = messagesByConv[conversation.id] ?? [];
  return messages[messages.length - 1]?.created_at ?? conversation.created_at;
}

function buildSection(
  id: SidebarConversationSectionId,
  title: SidebarConversationSection["title"],
  conversations: SidebarConversationItem[],
  options: {
    defaultExpanded: boolean;
    hideWhenEmpty: boolean;
    hasSearchQuery: boolean;
  },
): SidebarConversationSection {
  const forceExpandedByActive = conversations.some((item) => item.isActive);
  const forceExpandedBySearch = options.hasSearchQuery && conversations.length > 0;

  return {
    id,
    title,
    conversations,
    conversationIds: conversations.map((item) => item.id),
    defaultExpanded: options.defaultExpanded,
    forceExpandedByActive,
    forceExpandedBySearch,
    shouldRender: !options.hideWhenEmpty || conversations.length > 0,
  };
}

function compareLatestActivityDesc(
  a: SidebarConversationItem,
  b: SidebarConversationItem,
): number {
  return compareIsoDesc(a.latestActivityAt, b.latestActivityAt);
}

function compareIsoDesc(a: string | null, b: string | null): number {
  return timestampMs(b) - timestampMs(a);
}

function timestampMs(iso: string | null): number {
  if (!iso) return 0;
  const parsed = Date.parse(iso);
  return Number.isFinite(parsed) ? parsed : 0;
}
