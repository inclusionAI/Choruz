"use client";

import { lazy, Suspense, useCallback, useEffect, useRef, useState, type ImgHTMLAttributes } from "react";
import { useTransportUrl } from "../../hooks/use-transport-url";
import type { Components } from "react-markdown";
import { trace } from "../../lib/api/choruz-trace";
import type { Principal, ChatMessage } from "../../lib/api/choruz-types";
import type { QuotedMessage } from "../../lib/messages/quotes";
import { Avatar } from "../ui/avatar";
import { principalName, isAgent } from "../../lib/api/principals";
import { stripAnsi } from "../../lib/terminal/ansi";
import { formatFileSize } from "../../lib/format-bytes";

const ReactMarkdown = lazy(() => import("react-markdown"));
const remarkGfmModule = import("remark-gfm").then((m) => m.default);
let _remarkGfm: typeof import("remark-gfm").default | null = null;
remarkGfmModule.then((mod) => { _remarkGfm = mod; });

function absoluteTime(iso: string): string {
  const d = new Date(iso);
  const h = String(d.getHours()).padStart(2, "0");
  const m = String(d.getMinutes()).padStart(2, "0");
  return `${h}:${m}`;
}

/** Strip TUI characters (bullet symbols, box-drawing, progress lines, etc.) */
export function stripTuiChars(str: string): string {
  const lineFiltered = str
    .split("\n")
    .filter((line) => {
      const t = line.trim();
      if (t.length > 0 && /^[─━═\s]+$/.test(t)) return false;
      if (t === "❯" || t.startsWith("❯ ") || t === "›" || t.startsWith("› "))
        return false;
      if (t.length > 0 && /^[✽✻✳✢✶✦●·\s]+$/.test(t)) return false;
      if (/^[⎿└│]/.test(t)) return false;
      if (/^(Write|Read|Bash|Update|Edit|Glob|Grep)\(/.test(t)) return false;
      if (t.includes("tokens") && (/↓/.test(t) || /↑/.test(t))) return false;
      if (/^(esc |ctrl\+|\? for)/.test(t)) return false;
      if (t.startsWith("thinking with") || t.startsWith("thought for "))
        return false;
      if (t.startsWith("Reading ") || t.startsWith("Done (")) return false;
      return true;
    })
    .map((line) => line.replace(/^⏺\s*/, ""))
    .join("\n")
    .trim();
  return lineFiltered.replace(/[─━═]{3,}/g, "").trim();
}

/** Strip Choruz protocol tags */
export function stripChoruzTags(str: string): string {
  return str
    .replace(/\{\{CHORUZ_REPLY\s+(?:group|direct)=[^\}]*\}\}/g, "")
    .replace(/\{\{\/CHORUZ_REPLY\}\}/g, "")
    .replace(/\{\{CHORUZ_SHARE_FILE\s+[^\}]*\}\}/g, "")
    .replace(/\{\{CHORUZ_PROVISION\s+[^\}]*\}\}/g, "")
    .trim();
}

export function shouldGroup(prev: ChatMessage | null, curr: ChatMessage): boolean {
  if (!prev) return false;
  if (prev.sender_id !== curr.sender_id) return false;
  if (prev.content_type === "system" || curr.content_type === "system")
    return false;
  const d1 = new Date(prev.created_at).getTime();
  const d2 = new Date(curr.created_at).getTime();
  return Math.abs(d2 - d1) < 120_000;
}

// ---------------------------------------------------------------------------
// Attachment renderer
// ---------------------------------------------------------------------------

function AttachmentContent({ metadata }: { metadata: Record<string, unknown> }) {
  const mime = (metadata.mime_type as string) || "";
  const filename = (metadata.filename as string) || "file";
  const sizeBytes = (metadata.size_bytes as number) || 0;
  // The proxy supplies the authenticated principal's actor_id.
  const attachmentId = (metadata.attachment_id as string) || "";
  const downloadPath = (metadata.download_path as string) || "";
  const path = attachmentId
    ? `/api/attachments/${attachmentId}`
    : (downloadPath ? `/api${downloadPath}` : "");
  const preview = /^(image|video|audio)\//.test(mime) || mime === "application/pdf";
  const [requested, setRequested] = useState(false);
  const { url: downloadUrl, remote, error } = useTransportUrl(path, preview || requested);
  const downloaded = useRef(false);
  useEffect(() => {
    if (!requested || !downloadUrl || downloaded.current) return;
    downloaded.current = true;
    const link = document.createElement("a");
    link.href = downloadUrl;
    link.download = filename;
    link.click();
  }, [requested, downloadUrl, filename]);
  if (error) return <div className="msg-attachment" role="alert">{filename}: {error}</div>;

  // Image: render inline
  if (mime.startsWith("image/")) {
    return (
      <div className="msg-attachment msg-attachment-image">
        <a href={downloadUrl} download={remote ? filename : undefined} target="_blank" rel="noopener noreferrer">
          <img
            src={downloadUrl}
            alt={filename}
            className="msg-attachment-media"
            loading="lazy"
          />
        </a>
        <div className="msg-attachment-meta">
          {filename} ({formatFileSize(sizeBytes)})
        </div>
      </div>
    );
  }

  // Video: render player
  if (mime.startsWith("video/")) {
    return (
      <div className="msg-attachment msg-attachment-video">
        <video
          src={downloadUrl}
          controls
          className="msg-attachment-media"
          preload="metadata"
        />
        <div className="msg-attachment-meta">
          {filename} ({formatFileSize(sizeBytes)})
        </div>
      </div>
    );
  }

  // Audio: render player
  if (mime.startsWith("audio/")) {
    return (
      <div className="msg-attachment msg-attachment-audio">
        <audio src={downloadUrl} controls preload="metadata" className="msg-attachment-audio-control" />
        <div className="msg-attachment-meta">
          {filename} ({formatFileSize(sizeBytes)})
        </div>
      </div>
    );
  }

  // PDF: show icon + link
  if (mime === "application/pdf") {
    return (
      <div className="msg-attachment msg-attachment-file">
        <a href={downloadUrl} target="_blank" rel="noopener noreferrer" className="msg-attachment-link">
          <span className="msg-attachment-icon">PDF</span>
          <span className="msg-attachment-name">{filename}</span>
          <span className="msg-attachment-size">{formatFileSize(sizeBytes)}</span>
        </a>
      </div>
    );
  }

  // Other: generic file card
  return (
    <div className="msg-attachment msg-attachment-file">
      <a href={downloadUrl ?? "#"} target="_blank" rel="noopener noreferrer" className="msg-attachment-link"
        download={remote ? filename : undefined}
        onClick={(event) => { if (remote && !downloadUrl) { event.preventDefault(); setRequested(true); } }}>
        <span className="msg-attachment-icon">FILE</span>
        <span className="msg-attachment-name">{filename}</span>
        <span className="msg-attachment-size">{formatFileSize(sizeBytes)}</span>
      </a>
    </div>
  );
}

function AttachmentImage({ src, alt, ...props }: ImgHTMLAttributes<HTMLImageElement>) {
  const path = typeof src === "string" ? src.replace(/^\/v1\/attachments\//, "/api/attachments/") : undefined;
  const { url, error } = useTransportUrl(path);
  return error ? <span role="alert">{alt}: {error}</span> : <img {...props} src={url} alt={alt || ""} />;
}

const MARKDOWN_COMPONENTS: Components = {
  img: ({ node: _node, ...props }) => <AttachmentImage {...props} className="msg-inline-img" loading="lazy" />,
};

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export type ThreadRollupInfo = {
  replyCount: number;
  lastReplyAt: string;
  participantNames: string[];
};

export type MessageBubbleProps = {
  msg: ChatMessage;
  idx: number;
  allMsgs: ChatMessage[];
  principal: Principal;
  principals: Principal[];
  isTerminalChat: boolean;
  showRuntimeHost?: boolean;
  runtimeAccountName?: string;
  onAvatarClick?: (principalId: string) => void;
  onReply?: (msg: { id: string; senderName: string; content: string }) => void;
  onCreateTaskFromMessage?: (msg: ChatMessage) => void;
  /** "N replies" rollup shown under a thread root message. */
  threadRollup?: ThreadRollupInfo;
  /** On-demand-fetched quote targets (originals outside the loaded
   * history window), keyed by message id; "missing" = server 404. */
  quotedMessages?: ReadonlyMap<string, QuotedMessage>;
  /** Opens the thread side panel for this message (as thread root). */
  onOpenThread?: (rootId: string) => void;
  initialActionsOpen?: boolean;
  scrollToMessage: (msgId: string) => void;
  touchActiveId: string | null;
  onTouchStart: (msgId: string) => void;
  onTouchEnd: () => void;
  onTouchMove: () => void;
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function MessageBubble({
  msg,
  idx,
  allMsgs,
  principal,
  principals,
  isTerminalChat,
  showRuntimeHost = false,
  runtimeAccountName,
  onAvatarClick,
  onReply,
  onCreateTaskFromMessage,
  threadRollup,
  quotedMessages,
  onOpenThread,
  initialActionsOpen = false,
  scrollToMessage,
  touchActiveId,
  onTouchStart,
  onTouchEnd,
  onTouchMove,
}: MessageBubbleProps) {
  // System message
  if (msg.content_type === "system") {
    return (
      <div key={msg.id} className="msg-system">
        <span>{stripAnsi(msg.content)}</span>
      </div>
    );
  }

  // Terminal transcript message
  if (msg.content_type === "runtime_transcript") {
    return (
      <div key={msg.id} className="msg-terminal">
        <pre>{stripChoruzTags(stripAnsi(msg.content))}</pre>
      </div>
    );
  }

  const isSelf = msg.sender_id === principal.id;
  const isAgentMsg = isAgent(principals, msg.sender_id);
  const senderName = principalName(principal, principals, msg.sender_id);
  const prevMsg = idx > 0 ? allMsgs[idx - 1] : null;
  const isContinuation = shouldGroup(prevMsg, msg);
  const driverLabel = isAgentMsg ? "AI" : null;
  const onlineContext = msg.metadata.online_author_context as { owner_name?: string; harness?: string; account_name?: string } | undefined;
  const accountLabel = typeof onlineContext?.account_name === "string" ? onlineContext.account_name : runtimeAccountName;

  // In terminal mode, agent text responses render as terminal output too
  if (isTerminalChat && isAgentMsg && !isSelf) {
    return (
      <div key={msg.id} className="msg-terminal">
        <pre>{stripChoruzTags(stripAnsi(msg.content))}</pre>
      </div>
    );
  }

  // Quote block: resolve reply_to_id \u2014 prefer the loaded history (live,
  // jumpable), then the on-demand store (originals fetched from the DB
  // when outside the history window \u2014 WeChat/Feishu behavior).
  // Runtime-validate (metadata is Record<string, unknown> — a malformed
  // non-string value must skip the quote path, same guard as
  // collectMissingQuoteTargets in lib/messages/quotes.ts).
  const rawReplyTo = msg.metadata?.reply_to_id;
  const replyToId =
    typeof rawReplyTo === "string" && rawReplyTo.length > 0 ? rawReplyTo : undefined;
  let quoteBlock: React.ReactNode = null;
  if (replyToId) {
    const inHistory = allMsgs.find((m) => m.id === replyToId);
    const resolved: QuotedMessage | undefined =
      inHistory ?? quotedMessages?.get(replyToId);
    if (resolved && resolved !== "missing") {
      const replyMsg = resolved;
      const replySender = principalName(
        principal,
        principals,
        replyMsg.sender_id,
      );
      const quoteContent = stripAnsi(replyMsg.content);
      const preview = quoteContent.length > 80 ? quoteContent.slice(0, 80) + "…" : quoteContent;
      // Jump only works for messages in the loaded window; a fetched-on-
      // demand original still shows its content but isn't scroll-anchored.
      const jumpable = Boolean(inHistory);
      quoteBlock = (
        <div
          className="msg-quote"
          onClick={jumpable ? () => scrollToMessage(replyToId) : undefined}
          title={jumpable ? "Jump to original message" : undefined}
        >
          <span className="msg-quote-icon">{"\u21A9"}</span>
          <span className="msg-quote-sender">{replySender}</span>
          <span className="msg-quote-text">{preview}</span>
        </div>
      );
    } else if (resolved === "missing") {
      quoteBlock = (
        <div className="msg-quote msg-quote-missing">
          <span className="msg-quote-icon">{"\u21A9"}</span>
          <span className="msg-quote-text">[original message unavailable]</span>
        </div>
      );
    } else {
      // Fetch in flight (or store not wired, e.g. thread panel where the
      // root is always present) \u2014 neutral loading placeholder.
      quoteBlock = (
        <div className="msg-quote msg-quote-missing">
          <span className="msg-quote-icon">{"\u21A9"}</span>
          <span className="msg-quote-text">[loading original message\u2026]</span>
        </div>
      );
    }
  }

  const plainContent = stripAnsi(msg.content);
  const displayContent = isAgentMsg ? stripChoruzTags(stripTuiChars(plainContent)) : plainContent;
  const groupClass = [
    "msg-group",
    isSelf ? "self" : "",
    isAgentMsg && !isSelf ? "agent" : "",
    isContinuation ? "continuation" : "",
    touchActiveId === msg.id ? "touch-active" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div
      key={msg.id}
      className={groupClass}
      data-msg-id={msg.id}
      onTouchStart={() => onTouchStart(msg.id)}
      onTouchEnd={onTouchEnd}
      onTouchMove={onTouchMove}
    >
      <div
        className={`msg-avatar-col${!isSelf && onAvatarClick ? " clickable" : ""}`}
        onClick={
          !isSelf && onAvatarClick
            ? () => onAvatarClick(msg.sender_id)
            : undefined
        }
      >
        <Avatar name={senderName} size="small" />
      </div>
      <div className="msg-content-col">
        <div className="msg-sender">
          {senderName}
          {isAgentMsg && showRuntimeHost ? (
            <span className="msg-runtime-host" title="Machine running this Agent">
              {typeof msg.metadata.runtime_host_name === "string"
                ? msg.metadata.runtime_host_name
                : "This computer"}
              {accountLabel ? ` · ${accountLabel}` : ""}
            </span>
          ) : null}
          {driverLabel && (
            <span className="agent-badge">{driverLabel}</span>
          )}
          {typeof onlineContext?.harness === "string" && <span className="agent-badge">{onlineContext.harness.replace(/_/g, " ")}</span>}
          {typeof onlineContext?.owner_name === "string" && <span className="msg-runtime-host">Agent owner: {onlineContext.owner_name}</span>}
        </div>
        {quoteBlock}
        <div className="msg-bubble">
          {msg.content_type === "attachment" && msg.metadata?.attachment_id ? (
            <AttachmentContent metadata={msg.metadata} />
          ) : (
            <div className="msg-markdown">
              <Suspense fallback={<span>{displayContent}</span>}>
                <ReactMarkdown
                  remarkPlugins={_remarkGfm ? [_remarkGfm] : []}
                  components={MARKDOWN_COMPONENTS}
                >
                  {displayContent}
                </ReactMarkdown>
              </Suspense>
            </div>
          )}
          <span className="msg-time">{absoluteTime(msg.created_at)}</span>
        </div>
        {threadRollup && onOpenThread && (
          <button
            type="button"
            className="msg-thread-rollup"
            onClick={() => onOpenThread(msg.id)}
            title="Open thread"
          >
            <span className="msg-thread-rollup-avatars">
              {threadRollup.participantNames.slice(0, 3).map((name) => (
                <Avatar key={name} name={name} size="tiny" />
              ))}
            </span>
            <span className="msg-thread-rollup-count">
              {threadRollup.replyCount}{" "}
              {threadRollup.replyCount === 1 ? "reply" : "replies"}
            </span>
            <span className="msg-thread-rollup-time">
              {absoluteTime(threadRollup.lastReplyAt)}
            </span>
          </button>
        )}
      </div>
      {(onReply || onCreateTaskFromMessage || onOpenThread) && (
        <MessageActionsMenu
          msg={msg}
          senderName={senderName}
          onReply={onReply}
          onCreateTaskFromMessage={onCreateTaskFromMessage}
          onOpenThread={onOpenThread}
          initialOpen={initialActionsOpen}
        />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// MessageActionsMenu — Slack-style three-dot dropdown (Reply, Copy)
// ---------------------------------------------------------------------------

function MessageActionsMenu({
  msg,
  senderName,
  onReply,
  onCreateTaskFromMessage,
  onOpenThread,
  initialOpen = false,
}: {
  msg: ChatMessage;
  senderName: string;
  onReply?: (msg: { id: string; senderName: string; content: string }) => void;
  onCreateTaskFromMessage?: (msg: ChatMessage) => void;
  onOpenThread?: (rootId: string) => void;
  initialOpen?: boolean;
}) {
  const [open, setOpen] = useState(initialOpen);
  const containerRef = useRef<HTMLDivElement | null>(null);

  // Close on click outside or Escape
  useEffect(() => {
    if (!open) return;
    const onDocClick = (e: MouseEvent) => {
      if (!containerRef.current) return;
      if (!containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const handleReply = useCallback(() => {
    trace.event("reply_message", { msgId: msg.id, senderName });
    onReply?.({ id: msg.id, senderName, content: msg.content });
    setOpen(false);
  }, [msg, senderName, onReply]);

  const handleOpenThread = useCallback(() => {
    trace.event("reply_in_thread", { msgId: msg.id, senderName });
    onOpenThread?.(msg.id);
    setOpen(false);
  }, [msg, senderName, onOpenThread]);

  const handleCreateTask = useCallback(() => {
    trace.event("create_task_from_message_open", { msgId: msg.id, senderName });
    onCreateTaskFromMessage?.(msg);
    setOpen(false);
  }, [msg, senderName, onCreateTaskFromMessage]);

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(msg.content);
      trace.event("copy_message", { msgId: msg.id });
    } catch (err) {
      trace.event("copy_message_error", { msgId: msg.id, error: String(err) });
    }
    setOpen(false);
  }, [msg]);

  return (
    <div className="msg-actions" ref={containerRef}>
      <button
        type="button"
        className="msg-actions-btn"
        aria-label="Message actions"
        aria-haspopup="menu"
        aria-expanded={open}
        title="More actions"
        onClick={(e) => {
          e.stopPropagation();
          setOpen((v) => !v);
        }}
      >
        {"\u22EF"}
      </button>
      {open && (
        <div className="msg-actions-menu" role="menu">
          {onOpenThread ? (
            <button
              type="button"
              role="menuitem"
              className="msg-actions-menu-item"
              onClick={handleOpenThread}
            >
              Reply in thread
            </button>
          ) : null}
          {onReply ? (
            <button
              type="button"
              role="menuitem"
              className="msg-actions-menu-item"
              onClick={handleReply}
            >
              Reply
            </button>
          ) : null}
          {onCreateTaskFromMessage ? (
            <button
              type="button"
              role="menuitem"
              className="msg-actions-menu-item"
              onClick={handleCreateTask}
            >
              Create task
            </button>
          ) : null}
          <button
            type="button"
            role="menuitem"
            className="msg-actions-menu-item"
            onClick={handleCopy}
          >
            Copy
          </button>
        </div>
      )}
    </div>
  );
}
