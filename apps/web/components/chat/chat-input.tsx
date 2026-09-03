"use client";

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type KeyboardEvent,
} from "react";
import { Paperclip, X, ArrowUp } from "lucide-react";

import { trace } from "../../lib/api/choruz-trace";
import type { Principal, Conversation } from "../../lib/api/choruz-types";
import { Avatar } from "../ui/avatar";
import { principalName, isAgent } from "../../lib/api/principals";
import { formatFileSize } from "../../lib/format-bytes";
import { Spinner } from "../ui/spinner";

function draftStorageKey(principalId: string, conversationId: string): string {
  return `choruz:draft:${principalId}:${conversationId}`;
}

function readDraft(principalId: string, conversationId: string): string {
  try {
    return sessionStorage.getItem(draftStorageKey(principalId, conversationId)) ?? "";
  } catch {
    return "";
  }
}

function writeDraft(principalId: string, conversationId: string, content: string): void {
  try {
    if (content) sessionStorage.setItem(draftStorageKey(principalId, conversationId), content);
    else sessionStorage.removeItem(draftStorageKey(principalId, conversationId));
  } catch {
    // Storage can be unavailable in privacy-restricted browser contexts.
  }
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export type ReplyTo = {
  id: string;
  senderName: string;
  content: string;
};

export type ChatInputProps = {
  principal: Principal;
  agents: Principal[];
  activeConv: Conversation | null;
  placeholder: string;
  /** Called when the user sends the composer; attachments remain queued until this point. */
  onSendMessage: (content: string, attachments: File[]) => Promise<void>;
  replyTo?: ReplyTo | null;
  onCancelReply?: () => void;
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function ChatInput({
  principal,
  agents,
  activeConv,
  placeholder,
  onSendMessage,
  replyTo,
  onCancelReply,
}: ChatInputProps) {
  const [inputText, setInputText] = useState("");
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [mentionQuery, setMentionQuery] = useState<string | null>(null);
  const [mentionIdx, setMentionIdx] = useState(0);
  const [pendingFilesByConversation, setPendingFilesByConversation] = useState<Record<string, File[]>>({});
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const restoreFocusAfterSendRef = useRef(false);

  useEffect(() => {
    if (sending || !restoreFocusAfterSendRef.current) return;
    restoreFocusAfterSendRef.current = false;
    textareaRef.current?.focus();
  }, [sending]);
  const activeConversationIdRef = useRef(activeConv?.id);
  activeConversationIdRef.current = activeConv?.id;
  const pendingFiles = activeConv?.id ? pendingFilesByConversation[activeConv.id] ?? [] : [];

  // Mention candidates: only members of active conversation (excluding self) + @all for groups
  const mentionCandidates = useMemo(() => {
    const result: { id: string; name: string; type: string }[] = [];
    if (!activeConv) return result;

    if (activeConv.conversation_type === "group") {
      result.push({ id: "__all__", name: "all", type: "everyone" });
    }

    const seen = new Set<string>();
    for (const mid of Object.keys(activeConv.members)) {
      if (mid === principal.id) continue;
      if (seen.has(mid)) continue;
      seen.add(mid);
      const isAgentMember = isAgent(agents, mid);
      result.push({
        id: mid,
        name: principalName(principal, agents, mid),
        type: isAgentMember ? "agent" : "member",
      });
    }
    return result;
  }, [agents, activeConv, principal]);

  const filteredMentions = useMemo(() => {
    if (mentionQuery === null) return [];
    if (!mentionQuery) return mentionCandidates;
    const q = mentionQuery.toLowerCase();
    return mentionCandidates.filter((c) => c.name.toLowerCase().includes(q));
  }, [mentionQuery, mentionCandidates]);

  const sendMessage = useCallback(async () => {
    if ((!inputText.trim() && pendingFiles.length === 0) || sending) return;
    const sendingConversationId = activeConv?.id;
    if (!sendingConversationId) return;
    const draft = inputText;
    const content = draft.trim();
    const attachments = pendingFiles;
    setSending(true);
    setSendError(null);
    setInputText("");
    setMentionQuery(null);
    setPendingFilesByConversation((previous) => {
      const next = { ...previous };
      delete next[sendingConversationId];
      return next;
    });

    try {
      await onSendMessage(content, attachments);
      writeDraft(principal.id, sendingConversationId, "");
    } catch (err) {
      trace.event("chat_input_send_error", { error: String(err), contentLen: content.length });
      setPendingFilesByConversation((previous) => ({
        ...previous,
        [sendingConversationId]: attachments,
      }));
      if (activeConversationIdRef.current === sendingConversationId) {
        setInputText(draft);
        setSendError(err instanceof Error ? err.message : "Message failed to send. Try again.");
        writeDraft(principal.id, sendingConversationId, draft);
      }
    } finally {
      restoreFocusAfterSendRef.current =
        activeConversationIdRef.current === sendingConversationId;
      setSending(false);
    }
  }, [inputText, pendingFiles, sending, onSendMessage, activeConv?.id, principal.id]);

  const openFilePicker = useCallback(() => {
    if (sending) return;
    fileInputRef.current?.click();
  }, [sending]);

  const handleFileSelected = useCallback(
    (e: ChangeEvent<HTMLInputElement>) => {
      const files = Array.from(e.target.files ?? []);
      // Allow selecting the same file again later.
      e.target.value = "";
      if (files.length === 0 || !activeConv?.id || sending) return;
      setPendingFilesByConversation((previous) => ({
        ...previous,
        [activeConv.id]: [...(previous[activeConv.id] ?? []), ...files],
      }));
    },
    [activeConv?.id, sending],
  );

  const removePendingFile = useCallback((index: number) => {
    if (!activeConv?.id) return;
    setPendingFilesByConversation((previous) => {
      const files = previous[activeConv.id] ?? [];
      const nextFiles = files.filter((_, fileIndex) => fileIndex !== index);
      const next = { ...previous };
      if (nextFiles.length === 0) delete next[activeConv.id];
      else next[activeConv.id] = nextFiles;
      return next;
    });
  }, [activeConv?.id]);

  // Expose test API for browser automation (Safari bridge)
  useEffect(() => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const w = window as any;
    w.__choruz_sendMessage = (text: string) => {
      setInputText(text);
      // Schedule send after React state update
      setTimeout(async () => {
        setSending(true);
        try {
          await onSendMessage(text, []);
          setInputText("");
          setSendError(null);
        } catch (err) {
          setInputText(text);
          setSendError(err instanceof Error ? err.message : "Message failed to send. Try again.");
        } finally {
          setSending(false);
        }
      }, 50);
      return "queued";
    };
    return () => { delete w.__choruz_sendMessage; };
  }, [onSendMessage]);

  const handleInputChange = useCallback(
    (e: ChangeEvent<HTMLTextAreaElement>) => {
      const val = e.target.value;
      setInputText(val);
      setSendError(null);
      if (activeConv?.id) {
        writeDraft(principal.id, activeConv.id, val);
      }

      // Check for @mention
      const cursor = e.target.selectionStart ?? val.length;
      const beforeCursor = val.slice(0, cursor);
      const atMatch = beforeCursor.match(/@([^\s]*)$/);
      if (atMatch) {
        setMentionQuery(atMatch[1]);
        setMentionIdx(0);
      } else {
        setMentionQuery(null);
      }

      // Auto-resize textarea
      const ta = e.target;
      ta.style.height = "auto";
      ta.style.height = Math.min(ta.scrollHeight, 120) + "px";
    },
    [activeConv?.id, principal.id],
  );

  const insertMention = useCallback(
    (name: string) => {
      const ta = textareaRef.current;
      if (!ta) return;
      const cursor = ta.selectionStart ?? inputText.length;
      const before = inputText.slice(0, cursor);
      const after = inputText.slice(cursor);
      const atPos = before.lastIndexOf("@");
      if (atPos === -1) return;
      trace.event("mention_select", { agentName: name });
      const newText = before.slice(0, atPos) + `@${name} ` + after;
      setInputText(newText);
      if (activeConv?.id) {
        writeDraft(principal.id, activeConv.id, newText);
      }
      setMentionQuery(null);
      ta.focus();
      requestAnimationFrame(() => {
        const newCursorPos = atPos + name.length + 2;
        ta.setSelectionRange(newCursorPos, newCursorPos);
      });
    },
    [inputText, activeConv?.id, principal.id],
  );

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if (mentionQuery !== null && filteredMentions.length > 0) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setMentionIdx((i) => (i + 1) % filteredMentions.length);
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setMentionIdx(
            (i) => (i - 1 + filteredMentions.length) % filteredMentions.length,
          );
          return;
        }
        if (e.key === "Tab" || e.key === "Enter") {
          e.preventDefault();
          insertMention(filteredMentions[mentionIdx].name);
          return;
        }
        if (e.key === "Escape") {
          e.preventDefault();
          setMentionQuery(null);
          return;
        }
      }

      if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
        e.preventDefault();
        sendMessage();
      }
    },
    [mentionQuery, filteredMentions, mentionIdx, insertMention, sendMessage],
  );

  // Keep one draft per conversation, including across company switches.
  const convIdRef = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (activeConv?.id !== convIdRef.current) {
      convIdRef.current = activeConv?.id;
      const nextDraft = activeConv?.id
        ? readDraft(principal.id, activeConv.id)
        : "";
      setInputText(nextDraft);
      setSendError(null);
      setMentionQuery(null);
      if (onCancelReply) onCancelReply();
    }
  }, [activeConv?.id, onCancelReply, principal.id]);

  return (
    <div className="chat-input-bar">
      {replyTo && (
        <div className="reply-preview">
          <div className="reply-preview-body">
            <div className="reply-preview-label">
              <span className="reply-preview-icon">{"\u21A9"}</span>
              Replying to <strong>{replyTo.senderName}</strong>
            </div>
            <div className="reply-preview-text">
              {replyTo.content.length > 120
                ? replyTo.content.slice(0, 120) + "…"
                : replyTo.content}
            </div>
          </div>
          <button
            className="reply-preview-close"
            onClick={onCancelReply}
            title="Cancel reply"
          >
            <X size={14} aria-hidden="true" />
          </button>
        </div>
      )}
      {sendError && (
        <div className="reply-preview" role="alert" aria-live="assertive">
          <div className="reply-preview-body">
            <div className="reply-preview-label">Message not sent</div>
            <div className="reply-preview-text">{sendError}</div>
          </div>
          <button
            className="reply-preview-close"
            onClick={() => setSendError(null)}
            title="Dismiss send error"
          >
            <X size={14} aria-hidden="true" />
          </button>
        </div>
      )}
      {mentionQuery !== null && filteredMentions.length > 0 && (
        <div className="mention-dropdown">
          {filteredMentions.map((m, i) => (
            <div
              key={m.id}
              className={`mention-item${i === mentionIdx ? " highlighted" : ""}`}
              onMouseDown={(e) => {
                e.preventDefault();
                insertMention(m.name);
              }}
              onMouseEnter={() => setMentionIdx(i)}
            >
              <Avatar name={m.name} size="tiny" />
              <span className="mention-name">@{m.name}</span>
              <span className="mention-type">{m.type}</span>
            </div>
          ))}
        </div>
      )}
      {pendingFiles.length > 0 && (
        <div className="attachment-queue" aria-label="Attachments ready to send">
          {pendingFiles.map((file, index) => (
            <div className="attachment-queue-item" key={`${file.name}-${file.lastModified}-${index}`}>
              <span className="attachment-queue-icon" aria-hidden="true">{"\uD83D\uDCCE"}</span>
              <span className="attachment-queue-name" title={file.name}>{file.name}</span>
              <span className="attachment-queue-size">{formatFileSize(file.size)}</span>
              <button
                type="button"
                className="attachment-queue-remove"
                aria-label={`Remove ${file.name}`}
                title={`Remove ${file.name}`}
                onClick={() => removePendingFile(index)}
                disabled={sending}
              >
                <X size={12} aria-hidden="true" />
              </button>
            </div>
          ))}
        </div>
      )}
      <div className="chat-input-row">
        <input
          ref={fileInputRef}
          type="file"
          multiple
          onChange={handleFileSelected}
          style={{ display: "none" }}
          aria-label="Upload attachment"
        />
        <button
          className="attach-btn"
          onClick={openFilePicker}
          disabled={sending}
          title="Attach files"
          aria-label="Attach file"
        >
          <Paperclip size={18} strokeWidth={1.75} aria-hidden="true" />
        </button>
        <textarea
          ref={textareaRef}
          rows={1}
          placeholder={placeholder}
          value={inputText}
          onChange={handleInputChange}
          onKeyDown={handleKeyDown}
          disabled={sending}
        />
        <button
          className="send-btn"
          onClick={sendMessage}
          disabled={(!inputText.trim() && pendingFiles.length === 0) || sending}
          aria-busy={sending}
          title="Send message"
        >
          {sending ? <Spinner /> : <ArrowUp size={16} strokeWidth={2.25} aria-hidden="true" />}
        </button>
      </div>
    </div>
  );
}
