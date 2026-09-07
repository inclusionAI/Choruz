"use client";

import { X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { trace } from "../../lib/api/choruz-trace";
import type { ChatMessage, Conversation, Principal } from "../../lib/api/choruz-types";
import { MessageBubble } from "./message-bubble";
import { Spinner } from "../ui/spinner";
import { EmptyState } from "../ui/empty-state";
import { readDraft, writeDraft } from "../../lib/chat-drafts";

// ---------------------------------------------------------------------------
// ThreadPanel — Slack-style side panel for one message thread. Shows the root message, its replies, and a
// composer with an "Also send to #channel" checkbox (= metadata.broadcast).
// The reply list is derived from the same per-conversation message store as
// the main timeline, so WebSocket pushes update it with no extra plumbing.
// ---------------------------------------------------------------------------

export type ThreadPanelProps = {
  root: ChatMessage;
  replies: ChatMessage[];
  principal: Principal;
  principals: Principal[];
  activeConv: Conversation;
  /** True while the authoritative GET /threads/{root} fetch is in flight. */
  loading: boolean;
  error: string | null;
  navigationTarget?: string | null;
  onClose: () => void;
  /** Sends a threaded reply; `broadcast` mirrors the checkbox state. */
  onSendReply: (content: string, broadcast: boolean) => Promise<void>;
};

export function ThreadPanel({
  root,
  replies,
  principal,
  principals,
  activeConv,
  loading,
  error,
  navigationTarget,
  onClose,
  onSendReply,
}: ThreadPanelProps) {
  const draftScope = `${activeConv.id}:thread:${root.id}`;
  const [draft, setDraft] = useState(() => readDraft(principal.id, draftScope));
  const [broadcast, setBroadcast] = useState(false);
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const followingReplies = useRef(true);
  const [hasNewReplies, setHasNewReplies] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    const el = listRef.current;
    if (el && followingReplies.current) el.scrollTop = el.scrollHeight;
    if (!followingReplies.current) setHasNewReplies(true);
  }, [replies.length, root.id]);

  useEffect(() => {
    textareaRef.current?.focus();
  }, []);

  useEffect(() => {
    if (!navigationTarget || loading) return;
    const message = listRef.current?.querySelector(`[data-msg-id="${navigationTarget}"]`);
    if (!message) return;
    followingReplies.current = false;
    message.scrollIntoView({ block: "center" });
    message.classList.add("msg-highlight");
    const timer = setTimeout(() => message.classList.remove("msg-highlight"), 1500);
    return () => { clearTimeout(timer); message.classList.remove("msg-highlight"); };
  }, [navigationTarget, loading, replies]);

  const handleSend = useCallback(async () => {
    const content = draft.trim();
    if (!content || sending) return;
    setSending(true);
    setSendError(null);
    try {
      await onSendReply(content, broadcast);
      if (readDraft(principal.id, draftScope) === draft) writeDraft(principal.id, draftScope, "");
      setDraft("");
    } catch (err) {
      setSendError(err instanceof Error ? err.message : String(err));
    } finally {
      setSending(false);
    }
  }, [draft, broadcast, sending, onSendReply, principal.id, draftScope]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
        e.preventDefault();
        void handleSend();
      }
    },
    [handleSend],
  );

  const channelLabel = activeConv.name ? `#${activeConv.name}` : "channel";
  const noop = useCallback(() => {}, []);

  // Memoize the rendered messages: the composer's per-keystroke draft
  // state lives in this component, and without the memo every keystroke
  // would re-render (and re-parse markdown for) every reply bubble.
  const threadMsgNodes = useMemo(() => {
    const threadMsgs = [root, ...replies];
    return threadMsgs.map((msg, idx) => (
      <MessageBubble
        key={msg.id}
        msg={msg}
        idx={idx}
        allMsgs={threadMsgs}
        principal={principal}
        principals={principals}
        isTerminalChat={false}
        scrollToMessage={noop}
        touchActiveId={null}
        onTouchStart={noop}
        onTouchEnd={noop}
        onTouchMove={noop}
      />
    ));
  }, [root, replies, principal, principals, noop]);

  return (
    <aside className="thread-panel" aria-label="Thread">
      <div className="thread-panel-header">
        <div className="thread-panel-title">
          <span className="thread-panel-title-main">Thread</span>
          <span className="thread-panel-title-sub">{channelLabel}</span>
        </div>
        <button
          type="button"
          className="thread-panel-close"
          aria-label="Close thread"
          onClick={() => {
            trace.event("close_thread", { rootId: root.id });
            onClose();
          }}
        >
          <X size={16} aria-hidden="true" />
        </button>
      </div>

      <div className="thread-panel-messages" ref={listRef} onScroll={(event) => {
        const el = event.currentTarget;
        followingReplies.current = el.scrollHeight - el.scrollTop - el.clientHeight < 150;
        if (followingReplies.current) setHasNewReplies(false);
      }}>
        {threadMsgNodes}
        {repliesPlaceholder(replies.length, loading, error)}
        {error && <div className="thread-panel-error">{error}</div>}
      </div>

      <div className="thread-panel-composer">
        {hasNewReplies && <button type="button" className="thread-panel-send" onClick={() => {
          followingReplies.current = true;
          setHasNewReplies(false);
          if (listRef.current) listRef.current.scrollTop = listRef.current.scrollHeight;
        }}>New replies</button>}
        <textarea
          data-activity="thread_reply_draft"
          ref={textareaRef}
          value={draft}
          rows={2}
          aria-label="Reply in thread"
          placeholder="Reply in thread…"
          onChange={(e) => {
            setDraft(e.target.value);
            writeDraft(principal.id, draftScope, e.target.value);
          }}
          onKeyDown={handleKeyDown}
          disabled={sending}
        />
        <div className="thread-panel-composer-row">
          <label className="thread-panel-broadcast">
            <input
              type="checkbox"
              checked={broadcast}
              onChange={(e) => setBroadcast(e.target.checked)}
            />
            <span>Also send to {channelLabel}</span>
          </label>
          <button
            type="button"
            className="thread-panel-send"
            disabled={!draft.trim() || sending}
            onClick={() => void handleSend()}
          >
            {sending ? "Sending…" : "Send"}
          </button>
        </div>
        {sendError && <div className="thread-panel-error">{sendError}</div>}
      </div>
    </aside>
  );
}

/** Loading / empty-state placeholder shown below the thread's messages.
 * Suppressed while an error is shown — a failed replies fetch must not
 * assert "No replies yet" (an emptiness it could not verify). */
function repliesPlaceholder(
  replyCount: number,
  loading: boolean,
  error: string | null,
): React.ReactNode {
  if (replyCount > 0 || error) return null;
  if (loading) {
    return <div className="thread-panel-loading"><Spinner label="Loading thread…" /></div>;
  }
  return (
    <EmptyState inline description="No replies yet. Start the thread below." />
  );
}
