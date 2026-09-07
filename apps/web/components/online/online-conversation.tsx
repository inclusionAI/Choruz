"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { apiFetch, ApiRequestError } from "../../lib/api/choruz-api";
import type {
  ChatMessage,
  Conversation,
  Principal,
} from "../../lib/api/choruz-types";
import { ChatHeader } from "../chat/chat-header";
import { ChatInput } from "../chat/chat-input";
import { MessageList } from "../chat/message-list";
import { Modal } from "../ui/modal";
import { OnlineAgents } from "./online-agents";

export type OnlineGroup = {
  id: string;
  role: "host" | "guest";
  name: string;
  status: "pending" | "active" | "revoked";
  conversation_id: string;
  message_count?: number;
  latest_seq?: number;
  last_message?: { content: string; created_at: string } | null;
};
export type SharedMessage = {
  id: string;
  seq: number;
  sender_id: string;
  sender_name: string;
  agent: boolean;
  own: boolean;
  content: string;
  created_at: string;
  author_context?: {
    owner_name?: string | null;
    device_name?: string | null;
    account_name?: string | null;
    harness?: string | null;
  };
};
type Page = {
  messages: SharedMessage[];
  pending: { id: string; content: string }[];
  status: OnlineGroup["status"];
};
export const onlineConversationId = (id: string) => `online:${id}`;

export function onlineConversation(
  link: OnlineGroup,
  principal: Principal,
): Conversation {
  return {
    id: onlineConversationId(link.id),
    workspace_id: principal.workspace_id,
    conversation_type: "group",
    name: link.name,
    description: "Online group",
    avatar_url: null,
    creator_id: "",
    created_at: principal.created_at,
    updated_at: principal.updated_at,
    members: {},
  };
}

export function useOnlineGroups(sessionToken: string, principalId: string) {
  const [groups, setGroups] = useState<OnlineGroup[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [revision, setRevision] = useState(0);
  const [readCounts, setReadCounts] = useState<Record<string, number>>({});
  const storageKey = `choruz_online_reads:${principalId}`;
  useEffect(() => {
    try {
      const stored: unknown = JSON.parse(
        localStorage.getItem(storageKey) ?? "{}",
      );
      if (stored && typeof stored === "object" && !Array.isArray(stored))
        setReadCounts(
          Object.fromEntries(
            Object.entries(stored).filter(
              ([, value]) =>
                typeof value === "number" &&
                Number.isSafeInteger(value) &&
                value >= 0,
            ),
          ),
        );
    } catch {
      /* Storage is optional; messages remain server-owned. */
    }
  }, [storageKey]);
  const viewed = useCallback(
    (id: string, count: number) =>
      setReadCounts((previous) => {
        if ((previous[id] ?? 0) >= count) return previous;
        const next = { ...previous, [id]: count };
        try {
          localStorage.setItem(storageKey, JSON.stringify(next));
        } catch {
          /* Storage may be disabled. */
        }
        return next;
      }),
    [storageKey],
  );
  useEffect(() => {
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const load = async () => {
      try {
        const identity = await apiFetch<{ state: string }>(
          "/v1/online/session",
          sessionToken,
          { signal: abort.signal },
        );
        if (identity.state !== "signed_in") {
          if (!abort.signal.aborted) {
            setGroups([]);
            setError(null);
          }
          return;
        }
        const result = await apiFetch<{ groups: OnlineGroup[] }>(
          "/v1/online/groups",
          sessionToken,
          { signal: abort.signal },
        );
        if (!abort.signal.aborted) {
          setGroups(result.groups.filter((group) => group.role === "guest"));
          setError(null);
        }
      } catch (cause) {
        if (!abort.signal.aborted) {
          if (
            cause instanceof ApiRequestError &&
            [401, 403].includes(cause.status)
          ) {
            setGroups([]);
            setError(null);
          } else
            setError(
              cause instanceof Error
                ? cause.message
                : "Could not load Online groups",
            );
        }
      } finally {
        if (!abort.signal.aborted) {
          setLoading(false);
          timer = setTimeout(load, 3000);
        }
      }
    };
    void load();
    return () => {
      abort.abort();
      clearTimeout(timer);
    };
  }, [sessionToken, revision]);
  const unreads = Object.fromEntries(
    groups.map((group) => [
      onlineConversationId(group.id),
      {
        unread: Math.max(
          0,
          (group.message_count ?? 0) - (readCounts[group.id] ?? 0),
        ),
        mentions: 0,
        threadUnread: 0,
      },
    ]),
  );
  const previews = Object.fromEntries(
    groups.flatMap((group) =>
      group.last_message
        ? [[onlineConversationId(group.id), group.last_message]]
        : [],
    ),
  );
  return {
    groups,
    error,
    loading,
    unreads,
    previews,
    viewed,
    refresh: () => {
      setLoading(true);
      setRevision((value) => value + 1);
    },
  };
}

/** Group-only transport; the view never calls local conversation or runtime APIs. */
export function OnlineConversation({
  link,
  principal,
  sessionToken,
  onToggleSidebar,
  onManage,
  onViewed,
}: {
  link: OnlineGroup;
  principal: Principal;
  sessionToken: string;
  onToggleSidebar: () => void;
  onManage: () => void;
  onViewed: (id: string, count: number) => void;
}) {
  const [messages, setMessages] = useState<SharedMessage[]>([]);
  const [pending, setPending] = useState<Page["pending"]>([]);
  const [status, setStatus] = useState(link.status);
  const [error, setError] = useState<string | null>(null);
  const [files, setFiles] = useState<Record<string, File[]>>({});
  const [showDetails, setShowDetails] = useState(false);
  const retry = useRef<{ id: string; content: string } | null>(null);
  const retryKey = `choruz:online-send:${principal.id}:${link.id}`;
  const [revision, setRevision] = useState(0);
  const cursor = useRef(0);
  useEffect(() => {
    const mark = () => {
      if (
        document.visibilityState === "visible" &&
        cursor.current >= (link.latest_seq ?? Infinity)
      )
        onViewed(link.id, link.message_count ?? 0);
    };
    mark();
    document.addEventListener("visibilitychange", mark);
    return () => document.removeEventListener("visibilitychange", mark);
  }, [link.id, link.latest_seq, link.message_count, messages, onViewed]);
  useEffect(() => {
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const load = async () => {
      try {
        const result = await apiFetch<Page>(
          `/v1/online/groups/${link.id}/messages?after_seq=${cursor.current}`,
          sessionToken,
          { signal: abort.signal },
        );
        if (!abort.signal.aborted) {
          setStatus(result.status);
          setPending(result.pending);
          setError(null);
          setMessages((current) =>
            [
              ...new Map(
                [...current, ...result.messages].map((message) => [
                  message.id,
                  message,
                ]),
              ).values(),
            ].sort((a, b) => a.seq - b.seq),
          );
          if (result.messages.length)
            cursor.current = result.messages.at(-1)!.seq;
        }
      } catch (cause) {
        if (!abort.signal.aborted)
          setError(
            cause instanceof Error ? cause.message : "Could not load messages",
          );
      } finally {
        if (!abort.signal.aborted) timer = setTimeout(load, 1500);
      }
    };
    void load();
    return () => {
      abort.abort();
      clearTimeout(timer);
    };
  }, [link.id, sessionToken, revision]);
  const people = useMemo(
    () => [
      ...new Map(
        messages.map((message) => {
          const person: Principal = {
            ...principal,
            id: message.own ? principal.id : message.sender_id,
            name: message.sender_name,
            principal_type: message.agent ? "agent" : "human",
            scopes: [],
          };
          return [person.id, person] as const;
        }),
      ).values(),
    ],
    [messages, principal],
  );
  const conversation = useMemo(
    () => ({
      ...onlineConversation(link, principal),
      members: Object.fromEntries(
        people.map((person) => [
          person.id,
          { principal_id: person.id, joined_at: person.created_at },
        ]),
      ),
    }),
    [link, principal, people],
  );
  const chat: ChatMessage[] = messages.map((message) => ({
    id: message.id,
    workspace_id: principal.workspace_id,
    conversation_id: conversation.id,
    sender_id: message.own ? principal.id : message.sender_id,
    content: message.content,
    content_type: "text",
    metadata: {
      runtime_host_name:
        message.author_context?.device_name ?? "Device not shared",
      online_author_context: message.author_context,
    },
    server_seq: message.seq,
    idempotency_key: message.id,
    created_at: message.created_at,
    edited_at: null,
    edited_by: null,
  }));
  const accountNames = new Map(
    messages.flatMap((message) =>
      message.author_context?.account_name
        ? [[message.sender_id, message.author_context.account_name] as const]
        : [],
    ),
  );
  const sendMessage = async (content: string) => {
    if (!retry.current) {
      try {
        const saved = JSON.parse(sessionStorage.getItem(retryKey) ?? "null");
        if (
          saved &&
          typeof saved.id === "string" &&
          saved.content === content
        ) {
          retry.current = { id: saved.id, content };
        }
      } catch {
        /* A missing saved attempt starts a new send. */
      }
    }
    if (retry.current?.content !== content) {
      retry.current = { id: crypto.randomUUID(), content };
    }
    const outgoing = retry.current;
    try {
      sessionStorage.setItem(retryKey, JSON.stringify(outgoing));
    } catch {
      /* In-memory retries still retain their id. */
    }
    await apiFetch(`/v1/online/groups/${link.id}/messages`, sessionToken, {
      method: "POST",
      body: JSON.stringify(outgoing),
    });
    setPending((current) => [
      ...current.filter((message) => message.id !== outgoing.id),
      outgoing,
    ]);
    retry.current = null;
    try {
      sessionStorage.removeItem(retryKey);
    } catch {
      /* Storage may be disabled. */
    }
    setRevision((value) => value + 1);
  };
  return (
    <>
      <ChatHeader
        activeConv={conversation}
        chatTitle={link.name}
        chatSubtitle={`Online group · ${status}`}
        showDetail={showDetails}
        wsStatus="connected"
        onToggleSidebar={onToggleSidebar}
        onToggleDetail={() => setShowDetails((value) => !value)}
      />
      <section
        className="chat-with-thread online-conversation"
        aria-label="Online group conversation"
      >
        <div className="chat-primary">
          <p className="field-hint">
            Shared text and Agent replies. Files, terminals and private chats
            stay on the owner's device.
          </p>
          {status === "pending" && (
            <p role="status">Waiting for the group owner's device…</p>
          )}
          {status === "revoked" && (
            <p role="status">
              You left this group or your invitation was revoked. Saved history
              remains readable.
            </p>
          )}
          {error && (
            <p role="alert">
              {error}{" "}
              <button
                type="button"
                onClick={() => setRevision((value) => value + 1)}
              >
                Retry
              </button>
            </p>
          )}
          <MessageList
            messages={chat}
            principal={principal}
            principals={people}
            activeConv={conversation}
            isTerminalChat={false}
            thinkingAgents={new Set()}
            agentAccountNames={accountNames}
          />
          {pending.map((message) => (
            <p key={message.id} role="status">
              {message.content} — queued for delivery
            </p>
          ))}
          <ChatInput
            principal={principal}
            agents={people}
            activeConv={conversation}
            placeholder={`Message ${link.name}...`}
            disabled={status !== "active"}
            attachmentsEnabled={false}
            pendingFilesByConversation={files}
            setPendingFilesByConversation={setFiles}
            onSendMessage={sendMessage}
          />
        </div>
      </section>
      {showDetails && (
        <Modal
          title={link.name}
          description="Manage your Agents and view the participants in shared history."
          onClose={() => setShowDetails(false)}
        >
          <div className="modal-form">
            {status === "active" && <OnlineAgents linkId={link.id} sessionToken={sessionToken} />}
            {people.map((person) => {
              const context = messages
                .filter(
                  (message) =>
                    (message.own ? principal.id : message.sender_id) ===
                    person.id,
                )
                .at(-1)?.author_context;
              return (
                <div key={person.id}>
                  <strong>{person.name}</strong> ·{" "}
                  {person.principal_type === "agent" ? "Agent" : "Choruz user"}
                  {person.principal_type === "agent" && (
                    <p className="field-hint">
                      {context?.device_name ?? "Device not shared"}
                      {context?.account_name
                        ? ` · Harness account: ${context.account_name}`
                        : ""}
                      {context?.harness
                        ? ` · ${context.harness.replace(/_/g, " ")}`
                        : ""}
                    </p>
                  )}
                  {context?.owner_name && (
                    <p className="field-hint">
                      Agent owner: {context.owner_name}
                    </p>
                  )}
                </div>
              );
            })}
            <button
              type="button"
              className="btn-secondary"
              onClick={() => {
                setShowDetails(false);
                onManage();
              }}
            >
              Manage Online account and invitations
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}
