"use client";

import { MessageSquare } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import type { Principal, Conversation, ChatMessage } from "../../lib/api/choruz-types";
import { MessageBubble, shouldGroup, type ThreadRollupInfo } from "./message-bubble";
import type { QuotedMessage } from "../../lib/messages/quotes";
import { Avatar } from "../ui/avatar";
import { principalName, isAgent } from "../../lib/api/principals";
import { EmptyState } from "../ui/empty-state";
import { Spinner } from "../ui/spinner";

// ---------------------------------------------------------------------------
// Pretext height estimation (client-only)
// ---------------------------------------------------------------------------

type PreparedText = unknown;

let pretextPrepare: ((text: string, font: string) => PreparedText) | null = null;
let pretextLayout:
  | ((
      prepared: PreparedText,
      maxWidth: number,
      lineHeight: number,
    ) => { height: number; lineCount: number })
  | null = null;

if (typeof window !== "undefined") {
  import("@chenglou/pretext").then((mod) => {
    pretextPrepare = mod.prepare as typeof pretextPrepare;
    pretextLayout = mod.layout as typeof pretextLayout;
  });
}

const MAX_PREPARED_CACHE_ENTRIES = 1_000;
const preparedCache = new Map<string, { content: string; prepared: PreparedText }>();

const MSG_FONT = '14px -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif';
const MSG_LINE_HEIGHT = 21;

const BUBBLE_PADDING_V = 20;
const BUBBLE_PADDING_H = 28;
const SENDER_HEIGHT = 20;
const TIMESTAMP_HEIGHT = 16;
const QUOTE_BLOCK_HEIGHT = 40;
const THREAD_ROLLUP_HEIGHT = 32;
const MSG_GROUP_GAP = 6;
const DATE_SEP_HEIGHT = 38;
const SYSTEM_MSG_HEIGHT = 36;
const TERMINAL_BASE_HEIGHT = 34;
const AVATAR_COL_WIDTH = 46;

export function messageLayoutSignature(input: {
  containerWidth: number;
  content: string;
  contentType: string;
  attachmentMime: string;
  hasQuote: boolean;
  isContinuation: boolean;
  hasPrecedingDateSep: boolean;
  hasThreadRollup: boolean;
}): string {
  return JSON.stringify([
    input.containerWidth,
    input.content,
    input.contentType,
    input.attachmentMime,
    input.hasQuote,
    input.isContinuation,
    input.hasPrecedingDateSep,
    input.hasThreadRollup,
  ]);
}

export function estimateMessageHeight(
  msgId: string,
  content: string,
  containerWidth: number,
  hasQuote: boolean,
  isContinuation: boolean,
  contentType: string,
  hasPrecedingDateSep: boolean,
  hasThreadRollup = false,
  attachmentMime = "",
): number {
  let height = 0;

  if (hasPrecedingDateSep) {
    height += DATE_SEP_HEIGHT;
  }

  if (contentType === "system") {
    return height + SYSTEM_MSG_HEIGHT;
  }

  if (contentType === "runtime_transcript") {
    const lineCount = content.split("\n").length;
    return height + TERMINAL_BASE_HEIGHT + lineCount * 20;
  }

  if (contentType === "attachment") {
    const mediaHeight = attachmentMime.startsWith("image/") || attachmentMime.startsWith("video/")
      ? 300
      : attachmentMime.startsWith("audio/")
        ? 80
        : 64;
    return height
      + mediaHeight
      + TIMESTAMP_HEIGHT
      + (isContinuation ? 0 : SENDER_HEIGHT)
      + (hasQuote ? QUOTE_BLOCK_HEIGHT : 0)
      + (hasThreadRollup ? THREAD_ROLLUP_HEIGHT : 0)
      + MSG_GROUP_GAP;
  }

  const bubbleMaxWidth =
    Math.floor(containerWidth * 0.7) - AVATAR_COL_WIDTH - BUBBLE_PADDING_H;

  let textHeight: number;
  if (pretextPrepare && pretextLayout && bubbleMaxWidth > 0) {
    let cached = preparedCache.get(msgId);
    if (!cached || cached.content !== content) {
      const prepared = pretextPrepare(content, MSG_FONT);
      cached = { content, prepared };
      preparedCache.delete(msgId);
      preparedCache.set(msgId, cached);
      if (preparedCache.size > MAX_PREPARED_CACHE_ENTRIES) {
        preparedCache.delete(preparedCache.keys().next().value!);
      }
    } else {
      preparedCache.delete(msgId);
      preparedCache.set(msgId, cached);
    }
    const result = pretextLayout(cached.prepared, bubbleMaxWidth, MSG_LINE_HEIGHT);
    textHeight = result.height;
  } else {
    const charsPerLine = bubbleMaxWidth > 0 ? Math.max(Math.floor(bubbleMaxWidth / 8.5), 20) : 60;
    textHeight = Math.ceil(content.length / charsPerLine) * MSG_LINE_HEIGHT;
    textHeight = Math.max(textHeight, MSG_LINE_HEIGHT);
  }

  height += BUBBLE_PADDING_V + textHeight + TIMESTAMP_HEIGHT;

  if (!isContinuation) {
    height += SENDER_HEIGHT;
  }

  if (hasQuote) {
    height += QUOTE_BLOCK_HEIGHT;
  }

  if (hasThreadRollup) {
    height += THREAD_ROLLUP_HEIGHT;
  }

  height += MSG_GROUP_GAP;

  return height;
}

// ---------------------------------------------------------------------------
// Virtual scroll bookkeeping
// ---------------------------------------------------------------------------

const OVERSCAN = 5;

export function preservePrependScrollTop(
  previousTop: number,
  previousHeight: number,
  currentHeight: number,
): number {
  return Math.max(0, previousTop + currentHeight - previousHeight);
}

export function visibleRange(
  offsets: number[],
  scrollTop: number,
  viewportHeight: number,
): [number, number] {
  const total = offsets.length;
  if (total === 0) return [0, 0];

  let lo = 0;
  let hi = total - 1;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (offsets[mid] < scrollTop) {
      lo = mid + 1;
    } else {
      hi = mid;
    }
  }
  const start = Math.max(0, lo - OVERSCAN);

  const bottom = scrollTop + viewportHeight;
  lo = start;
  hi = total - 1;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    const topEdge = mid > 0 ? offsets[mid - 1] : 0;
    if (topEdge <= bottom) {
      lo = mid + 1;
    } else {
      hi = mid;
    }
  }
  const end = Math.min(total, lo + OVERSCAN);

  return [start, end];
}

function MeasuredMessageRow({
  messageId,
  layoutSignature,
  onHeight,
  children,
}: {
  messageId: string;
  layoutSignature: string;
  onHeight: (messageId: string, height: number, layoutSignature: string) => void;
  children: ReactNode;
}) {
  const rowRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const row = rowRef.current;
    if (!row) return;
    const report = () => onHeight(messageId, row.getBoundingClientRect().height, layoutSignature);
    report();
    const observer = new ResizeObserver(report);
    observer.observe(row);
    return () => observer.disconnect();
  }, [messageId, layoutSignature, onHeight]);
  return <div ref={rowRef} className="virtual-message-row">{children}</div>;
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export type MessageListProps = {
  messages: ChatMessage[];
  principal: Principal;
  agents: Principal[];
  activeConv: Conversation | null;
  isTerminalChat: boolean;
  thinkingAgents: Set<string>;
  agentAccountNames?: ReadonlyMap<string, string>;
  onAvatarClick?: (principalId: string) => void;
  onReply?: (msg: { id: string; senderName: string; content: string }) => void;
  onCreateTaskFromMessage?: (msg: ChatMessage) => void;
  /** Per-root thread rollups ("N replies" chips), keyed by root message id. */
  threadRollups?: Map<string, ThreadRollupInfo>;
  /** On-demand-fetched quote targets (see lib/messages/quotes.ts). */
  quotedMessages?: ReadonlyMap<string, QuotedMessage>;
  /** Opens the thread side panel for the given message id. */
  onOpenThread?: (rootId: string) => void;
  initialActionsOpen?: boolean;
  hasOlderMessages?: boolean;
  loadingOlderMessages?: boolean;
  onLoadOlderMessages?: () => Promise<void>;
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function MessageList({
  messages: rawMessages,
  principal,
  agents,
  activeConv,
  isTerminalChat,
  thinkingAgents,
  agentAccountNames,
  onAvatarClick,
  onReply,
  onCreateTaskFromMessage,
  threadRollups,
  quotedMessages,
  onOpenThread,
  initialActionsOpen = false,
  hasOlderMessages = false,
  loadingOlderMessages = false,
  onLoadOlderMessages,
}: MessageListProps) {
  // Deduplicate messages by id (IDB + polling can produce dupes)
  const messages = useMemo(() => {
    const seen = new Set<string>();
    return rawMessages.filter((m) => {
      if (seen.has(m.id)) return false;
      seen.add(m.id);
      return true;
    });
  }, [rawMessages]);

  const messagesEndRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const activeConversationIdRef = useRef(activeConv?.id);
  activeConversationIdRef.current = activeConv?.id;

  // ---- Mobile long-press for reply button ----
  const longPressTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [touchActiveId, setTouchActiveId] = useState<string | null>(null);

  const handleMsgTouchStart = useCallback((msgId: string) => {
    longPressTimerRef.current = setTimeout(() => {
      setTouchActiveId(msgId);
    }, 500);
  }, []);

  const handleMsgTouchEnd = useCallback(() => {
    if (longPressTimerRef.current) {
      clearTimeout(longPressTimerRef.current);
      longPressTimerRef.current = null;
    }
    setTimeout(() => setTouchActiveId(null), 3000);
  }, []);

  const handleMsgTouchMove = useCallback(() => {
    if (longPressTimerRef.current) {
      clearTimeout(longPressTimerRef.current);
      longPressTimerRef.current = null;
    }
  }, []);

  // ---- Virtual scroll state ----
  const [scrollTop, setScrollTop] = useState(0);
  const [containerHeight, setContainerHeight] = useState(0);
  const [containerWidth, setContainerWidth] = useState(0);
  const [measuredHeights, setMeasuredHeights] = useState<Record<string, { height: number; layoutSignature: string }>>({});

  const recordMeasuredHeight = useCallback((messageId: string, height: number, layoutSignature: string) => {
    setMeasuredHeights((previous) => {
      const measured = previous[messageId];
      if (measured?.layoutSignature === layoutSignature && Math.abs(measured.height - height) < 1) return previous;
      return { ...previous, [messageId]: { height, layoutSignature } };
    });
  }, []);

  useEffect(() => {
    const currentIds = new Set(messages.map((message) => message.id));
    setMeasuredHeights((previous) => {
      const entries = Object.entries(previous).filter(([messageId]) => currentIds.has(messageId));
      return entries.length === Object.keys(previous).length ? previous : Object.fromEntries(entries);
    });
  }, [messages]);

  const wasNearBottomRef = useRef(true);

  const isNearBottom = useCallback(() => {
    const el = containerRef.current;
    if (!el) return true;
    return el.scrollHeight - el.scrollTop - el.clientHeight < 150;
  }, []);

  const scrollToBottom = useCallback((behavior: ScrollBehavior = "smooth") => {
    requestAnimationFrame(() => {
      const el = containerRef.current;
      if (el) {
        el.scrollTop = el.scrollHeight;
      }
    });
  }, []);

  // ---- ResizeObserver for container dimensions ----
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const cr = entry.contentRect;
        setContainerHeight(cr.height);
        setContainerWidth(cr.width);
      }
    });
    ro.observe(el);
    setContainerHeight(el.clientHeight);
    setContainerWidth(el.clientWidth);
    return () => ro.disconnect();
  }, []);

  // ---- Scroll handler ----
  const onScroll = useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    setScrollTop(el.scrollTop);
    wasNearBottomRef.current = isNearBottom();
    if (
      el.scrollTop < 200 &&
      hasOlderMessages &&
      !loadingOlderMessages &&
      onLoadOlderMessages
    ) {
      const previousHeight = el.scrollHeight;
      const previousTop = el.scrollTop;
      const requestedConversationId = activeConv?.id;
      void onLoadOlderMessages().then(() => {
        requestAnimationFrame(() => {
          const current = containerRef.current;
          if (current && activeConversationIdRef.current === requestedConversationId) {
            current.scrollTop = preservePrependScrollTop(
              previousTop,
              previousHeight,
              current.scrollHeight,
            );
          }
        });
      }).catch(() => {});
    }
  }, [activeConv?.id, hasOlderMessages, isNearBottom, loadingOlderMessages, onLoadOlderMessages]);

  // ---- Compute per-message heights & cumulative offsets ----
  const { offsets, totalHeight, layoutSignatures } = useMemo(() => {
    const offs: number[] = new Array(messages.length);
    const signatures: string[] = new Array(messages.length);
    let cumulative = 0;
    for (let i = 0; i < messages.length; i++) {
      const msg = messages[i];
      const prevMsg = i > 0 ? messages[i - 1] : null;

      const msgDate = new Date(msg.created_at).toDateString();
      const prevDate = prevMsg
        ? new Date(prevMsg.created_at).toDateString()
        : null;
      const hasPrecedingDateSep = i === 0 || msgDate !== prevDate;

      const isContinuation = shouldGroup(prevMsg, msg);

      const hasQuote = !!msg.metadata?.reply_to_id;

      let effectiveType = msg.content_type;
      if (
        isTerminalChat &&
        effectiveType !== "system" &&
        effectiveType !== "runtime_transcript" &&
        isAgent(agents, msg.sender_id) &&
        msg.sender_id !== principal.id
      ) {
        effectiveType = "runtime_transcript";
      }

      const hasThreadRollup = Boolean(threadRollups?.has(msg.id));
      const attachmentMime = typeof msg.metadata?.mime_type === "string" ? msg.metadata.mime_type : "";
      const layoutSignature = messageLayoutSignature({
        containerWidth,
        content: msg.content,
        contentType: effectiveType,
        attachmentMime,
        hasQuote,
        isContinuation,
        hasPrecedingDateSep,
        hasThreadRollup,
      });
      signatures[i] = layoutSignature;
      const estimatedHeight = estimateMessageHeight(
        msg.id,
        msg.content,
        containerWidth,
        hasQuote,
        isContinuation,
        effectiveType,
        hasPrecedingDateSep,
        hasThreadRollup,
        attachmentMime,
      );
      const measured = measuredHeights[msg.id];
      const h = measured?.layoutSignature === layoutSignature ? measured.height : estimatedHeight;
      cumulative += h;
      offs[i] = cumulative;
    }
    return { offsets: offs, totalHeight: cumulative, layoutSignatures: signatures };
  }, [messages, containerWidth, isTerminalChat, agents, principal.id, threadRollups, measuredHeights]);

  // ---- Determine visible window ----
  const [startIdx, endIdx] = useMemo(
    () => visibleRange(offsets, scrollTop, containerHeight),
    [offsets, scrollTop, containerHeight],
  );

  // ---- Conversation switch: always scroll to bottom ----
  const prevConvRef = useRef(activeConv?.id);
  useEffect(() => {
    if (activeConv?.id !== prevConvRef.current) {
      prevConvRef.current = activeConv?.id;
      wasNearBottomRef.current = true;
      scrollToBottom("instant" as ScrollBehavior);
      return;
    }
    if (wasNearBottomRef.current) {
      scrollToBottom();
    }
  }, [messages.length, activeConv?.id, scrollToBottom]);

  // ---- scrollToMessage ----
  const scrollToMessage = useCallback(
    (msgId: string) => {
      const idx = messages.findIndex((m) => m.id === msgId);
      if (idx < 0) return;
      const el = containerRef.current;
      if (!el) return;
      const targetTop = idx > 0 ? offsets[idx - 1] : 0;
      const targetCenter = targetTop - containerHeight / 2;
      el.scrollTo({ top: Math.max(0, targetCenter), behavior: "smooth" });
      requestAnimationFrame(() => {
        setTimeout(() => {
          const domEl = el.querySelector(`[data-msg-id="${msgId}"]`);
          if (domEl) {
            domEl.classList.add("msg-highlight");
            setTimeout(() => domEl.classList.remove("msg-highlight"), 1500);
          }
        }, 400);
      });
    },
    [messages, offsets, containerHeight],
  );

  // ---- Build the visible slice of rendered messages ----
  const renderedItems = useMemo(() => {
    if (messages.length === 0) return null;

    const items: React.ReactNode[] = [];
    for (let i = startIdx; i < endIdx && i < messages.length; i++) {
      const msg = messages[i];
      const msgDate = new Date(msg.created_at).toDateString();
      const prevDate =
        i > 0 ? new Date(messages[i - 1].created_at).toDateString() : null;
      const dateSeparator = i === 0 || msgDate !== prevDate
        ? <div className="msg-date-sep">
            <span>
              {new Date(msg.created_at).toLocaleDateString("en-US", {
                weekday: "short",
                month: "short",
                day: "numeric",
              })}
            </span>
          </div>
        : null;
      items.push(
        <MeasuredMessageRow
          key={msg.id}
          messageId={msg.id}
          layoutSignature={layoutSignatures[i]}
          onHeight={recordMeasuredHeight}
        >
          {dateSeparator}
          <MessageBubble
          msg={msg}
          idx={i}
          allMsgs={messages}
          principal={principal}
          agents={agents}
          isTerminalChat={isTerminalChat}
          showRuntimeHost={activeConv?.conversation_type === "group"}
          runtimeAccountName={agentAccountNames?.get(msg.sender_id)}
          onAvatarClick={onAvatarClick}
          onReply={onReply}
          onCreateTaskFromMessage={onCreateTaskFromMessage}
          threadRollup={threadRollups?.get(msg.id)}
          quotedMessages={quotedMessages}
          onOpenThread={onOpenThread}
          initialActionsOpen={initialActionsOpen}
          scrollToMessage={scrollToMessage}
          touchActiveId={touchActiveId}
          onTouchStart={handleMsgTouchStart}
          onTouchEnd={handleMsgTouchEnd}
          onTouchMove={handleMsgTouchMove}
          />
        </MeasuredMessageRow>,
      );
    }
    return items;
  }, [messages, startIdx, endIdx, principal, agents, activeConv?.conversation_type, isTerminalChat, agentAccountNames, onAvatarClick, onReply, onCreateTaskFromMessage, threadRollups, quotedMessages, onOpenThread, initialActionsOpen, scrollToMessage, touchActiveId, handleMsgTouchStart, handleMsgTouchEnd, handleMsgTouchMove, recordMeasuredHeight, layoutSignatures]);

  // Spacer heights
  const topSpacer = startIdx > 0 && offsets.length > 0 ? offsets[startIdx - 1] : 0;
  const bottomSpacer =
    endIdx < offsets.length ? totalHeight - offsets[endIdx - 1] : 0;

  return (
    <div className="messages-area" ref={containerRef} onScroll={onScroll}>
      {messages.length === 0 ? (
        <EmptyState
          icon={<MessageSquare size={40} strokeWidth={1.25} />}
          title="No messages yet"
          description="Send the first message to start the conversation."
        />
      ) : (
        <div style={{ position: "relative", display: "flex", flexDirection: "column", gap: "2px" }}>
          {loadingOlderMessages && (
            <div className="messages-history-loading"><Spinner label="Loading older messages…" /></div>
          )}
          {topSpacer > 0 && (
            <div style={{ height: topSpacer, pointerEvents: "none" }} />
          )}

          {renderedItems}

          {thinkingAgents.size > 0 &&
            activeConv?.conversation_type === "group" &&
            Array.from(thinkingAgents)
              .filter((id) => activeConv.members[id])
              .map((id) => {
                const name = principalName(principal, agents, id);
                return (
                <div key={`thinking-${id}`} className="msg-thinking">
                  <Avatar name={name} size="small" />
                  <span className="thinking-name">{name}</span>
                  <span className="thinking-dots">
                    <span />
                    <span />
                    <span />
                  </span>
                </div>
                );
              })}

          {bottomSpacer > 0 && (
            <div style={{ height: bottomSpacer, pointerEvents: "none" }} />
          )}

          <div ref={messagesEndRef} />
        </div>
      )}
    </div>
  );
}
