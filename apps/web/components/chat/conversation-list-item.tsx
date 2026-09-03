"use client";

import { Archive, ArchiveRestore, EyeOff, Pin, PinOff } from "lucide-react";
import type { Principal, Conversation, ChatMessage } from "../../lib/api/choruz-types";
import { Avatar } from "../ui/avatar";
import {
  conversationPreviewText,
} from "../../lib/messages/sidebar-conversations";
import { conversationDisplayName } from "../../lib/api/principals";

function relativeTime(iso: string): string {
  const date = new Date(iso);
  const now = Date.now();
  const diff = now - date.getTime();
  if (diff < 0) return "now";
  const seconds = Math.floor(diff / 1000);
  if (seconds < 10) return "now";
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d`;
  const mon = date.getMonth() + 1;
  const day = date.getDate();
  return `${mon}/${day}`;
}

function truncate(str: string, max: number): string {
  if (str.length <= max) return str;
  return str.slice(0, max) + "…";
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export type ConversationListItemProps = {
  conv: Conversation;
  principal: Principal;
  agents: Principal[];
  messages: ChatMessage[];
  isActive: boolean;
  isSelected: boolean;
  isPinned: boolean;
  isArchived: boolean;
  pinPending: boolean;
  archivePending: boolean;
  hidePending: boolean;
  manageMode: boolean;
  /** Server-side unread message count (Mattermost pattern). */
  unreadCount: number;
  /** Server-side mention count. */
  mentionCount: number;
  /** Server-side count of THREADS with unread replies (DISTINCT threads,
   * not individual replies). Threads clear independently of the
   * conversation, via POST /threads/{root}/view. */
  threadUnreadCount?: number;
  hideEmptyPreview?: boolean;
  onSelect: () => void;
  onToggleSelection: () => void;
  onTogglePin: (nextPinned: boolean) => void;
  onToggleArchive: (nextArchived: boolean) => void;
  onHide: () => void;
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function ConversationListItem({
  conv,
  principal,
  agents,
  messages,
  isActive,
  isSelected,
  isPinned,
  isArchived,
  pinPending,
  archivePending,
  hidePending,
  manageMode,
  unreadCount,
  mentionCount,
  threadUnreadCount = 0,
  hideEmptyPreview = false,
  onSelect,
  onToggleSelection,
  onTogglePin,
  onToggleArchive,
  onHide,
}: ConversationListItemProps) {
  const name = conversationDisplayName(conv, principal, agents);
  const lastMsg = messages[messages.length - 1];
  const preview = conversationPreviewText({
    messages,
    principal,
    agents,
    hideEmptyPlaceholder: hideEmptyPreview,
  });
  const time = lastMsg ? relativeTime(lastMsg.created_at) : "";

  const handleClick = () => {
    if (manageMode) {
      onToggleSelection();
    } else {
      onSelect();
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      handleClick();
    }
  };

  const handlePinClick = (e: React.MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
    e.stopPropagation();
    onTogglePin(!isPinned);
  };

  const handleArchiveClick = (e: React.MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
    e.stopPropagation();
    const list = e.currentTarget.closest(".conversation-list");
    const items = Array.from(list?.querySelectorAll<HTMLElement>(".conv-item-main") ?? []);
    const currentItem = e.currentTarget.closest(".conv-item")?.querySelector<HTMLElement>(".conv-item-main");
    const currentIndex = currentItem ? items.indexOf(currentItem) : -1;
    const fallbackId = currentIndex >= 0
      ? items[currentIndex + 1]?.dataset.conversationId ?? items[currentIndex - 1]?.dataset.conversationId
      : undefined;
    onToggleArchive(!isArchived);
    requestAnimationFrame(() => {
      const sameItem = document.querySelector<HTMLElement>(`[data-conversation-id="${conv.id}"]`);
      const fallbackItem = fallbackId
        ? document.querySelector<HTMLElement>(`[data-conversation-id="${fallbackId}"]`)
        : null;
      (sameItem ?? fallbackItem ?? document.querySelector<HTMLElement>(".conversation-section-header"))?.focus();
    });
  };

  const handleHideClick = (e: React.MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
    e.stopPropagation();
    onHide();
  };

  const otherMemberId = conv.conversation_type === "direct"
    ? Object.keys(conv.members).find((id) => id !== principal.id)
    : undefined;
  const isAgentDirectMessage = Boolean(
    otherMemberId && agents.some((agent) => agent.id === otherMemberId),
  );

  return (
    <div
      role="listitem"
      className={`conv-item${isActive && !manageMode ? " active" : ""}${isSelected ? " selected" : ""}${isPinned ? " pinned" : ""}`}
    >
      {manageMode && (
        <input
          type="checkbox"
          checked={isSelected}
          onChange={onToggleSelection}
          onClick={(e) => e.stopPropagation()}
          className="conv-select-checkbox"
        />
      )}
      <div
        role="button"
        tabIndex={0}
        data-conversation-id={conv.id}
        aria-current={isActive && !manageMode ? "true" : undefined}
        className="conv-item-main"
        onClick={handleClick}
        onKeyDown={handleKeyDown}
      >
        <Avatar name={name} />
        {/* AI badge for agent DMs */}
        {conv.conversation_type === "direct" && (() => {
          return isAgentDirectMessage ? (
            <span className="conv-ai-badge">AI</span>
          ) : null;
        })()}
        <div className="conv-meta">
          <span className="conv-name">
            {conv.conversation_type === "group" && (
              <span className="conv-channel-prefix">#</span>
            )}
            {name}
          </span>
          {preview && <span className="conv-preview">{truncate(preview, 50)}</span>}
        </div>
        {!manageMode && time && <span className="conv-time" suppressHydrationWarning>{time}</span>}
        {/* Unread badge (server-side count) */}
        {!manageMode && unreadCount > 0 && (
          <span className="conv-unread">{unreadCount > 99 ? "99+" : unreadCount}</span>
        )}
        {/* Thread unread badge: outlined (vs filled) — thread replies clear
            per thread, not by opening the conversation */}
        {!manageMode && threadUnreadCount > 0 && (
          <span className="conv-thread-unread" title="Threads with unread replies">
            {threadUnreadCount > 99 ? "99+" : threadUnreadCount}
          </span>
        )}
        {manageMode && conv.conversation_type === "group" && (
          <span className="conv-kind-badge">group</span>
        )}
      </div>
      {!manageMode && (
        <div className="conv-item-actions">
          {!isArchived && (
            <button
              type="button"
              className={`conv-item-action-btn conv-pin-btn${isPinned ? " active" : ""}${pinPending ? " pending" : ""}`}
              aria-label={isPinned ? `Unpin chat: ${name}` : `Pin chat: ${name}`}
              aria-pressed={isPinned}
              aria-busy={pinPending}
              title={isPinned ? "Unpin chat" : "Pin chat"}
              disabled={pinPending || archivePending || hidePending}
              onClick={handlePinClick}
            >
              {isPinned ? (
                <PinOff size={15} aria-hidden="true" />
              ) : (
                <Pin size={15} aria-hidden="true" />
              )}
            </button>
          )}
          <button
            type="button"
            className={`conv-item-action-btn conv-archive-btn${isArchived ? " active" : ""}${archivePending ? " pending" : ""}`}
            aria-label={isArchived ? `Restore chat: ${name}` : `Archive chat: ${name}`}
            aria-pressed={isArchived}
            aria-busy={archivePending}
            title={isArchived ? "Restore chat" : "Archive chat"}
            disabled={archivePending || pinPending || hidePending}
            onClick={handleArchiveClick}
          >
            {isArchived ? (
              <ArchiveRestore size={15} aria-hidden="true" />
            ) : (
              <Archive size={15} aria-hidden="true" />
            )}
          </button>
          {isAgentDirectMessage && !isArchived && (
            <button
              type="button"
              className={`conv-item-action-btn conv-hide-btn${hidePending ? " pending" : ""}`}
              aria-label={`Hide session: ${name}`}
              aria-busy={hidePending}
              title="Hide session"
              disabled={hidePending || pinPending || archivePending}
              onClick={handleHideClick}
            >
              <EyeOff size={15} aria-hidden="true" />
            </button>
          )}
        </div>
      )}
    </div>
  );
}
