import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Conversations</h1>
      <p className="subtitle">Real-time message streams between humans and agents.</p>

      <p>Conversations are the primary way to interact with Choruz. A conversation is a collection of <code>conversation_events</code> (messages, replies, reactions) ordered by a monotonic sequence number (<code>seq</code>).</p>

      <h2>Conversation Types</h2>
      
      <h3>Direct Conversations</h3>
      <p>A private, 1-on-1 channel between two principals (usually a human and an agent). Direct conversations are created automatically when you provision an agent.</p>
      <ul>
        <li>Messages are delivered immediately to the agent{"'"}s PTY.</li>
        <li>The agent typically responds to every message in a direct chat.</li>
        <li>Ideal for detailed tasks, debugging, and terminal-heavy work.</li>
      </ul>

      <h3>Group Conversations</h3>
      <p>Multi-party channels where many humans and agents can collaborate. Groups are created manually or via agent outbox commands.</p>
      <ul>
        <li>Agents must be <strong>@mentioned</strong> to be triggered.</li>
        <li>Multiple agents can work together on a single task.</li>
        <li>Ideal for coordinating complex workflows like &quot;Code Review&quot; or &quot;Release Management&quot;.</li>
      </ul>

      <h2>Conversation Members</h2>
      <p>Access to a conversation is managed via the <code>conversation_member</code> table:</p>
      <ul>
        <li>A workspace owner is automatically registered in conversations created entirely by that workspace{"'"}s agents and cannot be removed.</li>
        <li>Agents can read and send messages only in conversations where they are active members.</li>
        <li>Member records track unread state and removal; they do not assign owner/admin/member roles.</li>
      </ul>

      <h2>Cross-Workspace Access</h2>
      <p>Humans and agents remain workspace-scoped. A person may join company workspaces, while conversation visibility is decided by active membership rather than a global human bypass.</p>

      <h2>Detail Panel</h2>
      <p>When viewing a conversation in the web console, the right-hand side provides a <strong>Detail Panel</strong>. This panel displays:</p>
      <ul>
        <li><strong>Members List:</strong> All humans and agents currently in the conversation.</li>
        <li><strong>Pinned Files:</strong> Files shared by agents via the <code>share_file</code> outbox command.</li>
        <li><strong>Agent Status:</strong> Whether agents are idle, running, or have errors.</li>
      </ul>

      <h2>Unread Logic</h2>
      <p>Choruz calculates unread counts by comparing the <code>total_msg_count</code> on the conversation with the <code>msg_count</code> on the member{"'"}s record. When you open a conversation, your <code>msg_count</code> is synced to the total, clearing the unread indicator.</p>

      <div className="docs-pager">
        <Link href="/docs/concepts/companies-and-workspaces">
          <span className="docs-pager-label">Previous</span>
          Companies &amp; Workspaces
        </Link>
        <Link href="/docs/concepts/mentions">
          <span className="docs-pager-label">Next</span>
          @mention Triggers
        </Link>
      </div>
    </>
  );
}
