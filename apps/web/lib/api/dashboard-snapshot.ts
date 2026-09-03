import type {
  Company,
  ConsoleSnapshot,
  DashboardBootstrap,
  RuntimeBindingInfo,
} from "./choruz-types";

/** Everything `ChatApp` needs to mount, derived from one `/v1/bootstrap` page. */
export type DashboardSnapshotProps = {
  initialSnapshot: ConsoleSnapshot;
  initialCompanies: Company[];
  runtimeBindings: RuntimeBindingInfo[];
  initialSyncCursor: number;
  initialBootstrapNextCursor: string | null;
  initialBootstrapHasMore: boolean;
};

export const EMPTY_SNAPSHOT: ConsoleSnapshot = {
  principal: {
    id: "",
    workspace_id: "",
    principal_type: "human",
    name: "",
    avatar_url: null,
    scopes: [],
    disabled: false,
    created_at: "",
    updated_at: "",
  },
  conversations: [],
  messages_by_conversation: {},
  agents: [],
  audit_logs: [],
};

/** The shell renders from this when the bootstrap fails. */
export const EMPTY_DASHBOARD_PROPS: DashboardSnapshotProps = {
  initialSnapshot: EMPTY_SNAPSHOT,
  initialCompanies: [],
  runtimeBindings: [],
  initialSyncCursor: 0,
  initialBootstrapNextCursor: null,
  initialBootstrapHasMore: false,
};

export function dashboardSnapshotFromBootstrap(bootstrap: DashboardBootstrap): DashboardSnapshotProps {
  const items = bootstrap.conversations.items;
  return {
    initialSnapshot: {
      principal: bootstrap.principal,
      principals: bootstrap.principals,
      conversations: items.map((item) => item.conversation),
      messages_by_conversation: Object.fromEntries(
        items
          .filter((item) => item.last_message)
          .map((item) => [item.conversation.id, [item.last_message!]]),
      ),
      agents: bootstrap.agents,
      audit_logs: [],
      plugins: bootstrap.plugins,
      pinned_conversations: items
        .filter((item) => item.pinned_at)
        .map((item) => ({ conversation_id: item.conversation.id, pinned_at: item.pinned_at! })),
      archived_conversations: items
        .filter((item) => item.archived_at)
        .map((item) => ({ conversation_id: item.conversation.id, archived_at: item.archived_at! })),
      hidden_conversations: bootstrap.hidden_conversations,
      unreads: items.map((item) => ({
        conversation_id: item.conversation.id,
        unread_count: item.unread_count,
        mention_count: item.mention_count,
        thread_unread_count: item.thread_unread_count,
      })),
    },
    initialCompanies: bootstrap.companies,
    runtimeBindings: bootstrap.runtime_bindings,
    initialSyncCursor: bootstrap.sync_cursor,
    initialBootstrapNextCursor: bootstrap.conversations.next_cursor,
    initialBootstrapHasMore: bootstrap.conversations.has_more,
  };
}
