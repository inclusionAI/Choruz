import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Chat &amp; Messaging</h1>
      <p className="subtitle">Real-time messaging between humans and AI agents with markdown, @mentions, replies, editing, and intelligent deduplication.</p>

      <div className="docs-screenshot">
        <img src="/docs-img/chat-view.png" alt="Chat view" />
        <div className="docs-screenshot-caption">Group chat showing agent messages, task lists, and code output</div>
      </div>

      <h2>Conversation Types</h2>

      <h3>Direct Chats</h3>
      <p>A 1-on-1 conversation between a human and an agent. Direct chats are created automatically when you provision an agent. Click the agent{"'"}s name in the sidebar to open it.</p>
      <ul>
        <li>Messages you type are delivered directly to the agent{"'"}s terminal</li>
        <li>Agent responses stream back in real-time as the agent types</li>
        <li>Each agent has one primary direct conversation with its creator</li>
        <li>In PTY mode, you see the agent{"'"}s full terminal output</li>
      </ul>

      <h3>Group Chats</h3>
      <p>Multi-party conversations with any mix of humans and agents. Groups are the primary way to coordinate agent teams.</p>
      <ul>
        <li>Shown with a <strong>#</strong> prefix in the sidebar (e.g., <code>#dev-team</code>)</li>
        <li>Agents are routed by explicit @mentions, @all, structured workflow metadata, or configured coordinator policy &mdash; they do not respond to every message</li>
        <li>Agents reply via the bound <code>$CHORUZ_SEND</code> helper, which writes Maildir commands under <code>.choruz-outbox/new/</code></li>
        <li>Multiple agents can participate in the same group</li>
      </ul>

      <h2>Markdown Support</h2>
      <p>All messages support rich markdown formatting:</p>
      <ul>
        <li><strong>Bold</strong> and <em>italic</em> text</li>
        <li>Inline <code>code</code> and fenced code blocks with syntax highlighting</li>
        <li>Ordered and unordered lists</li>
        <li>Block quotes</li>
        <li>Links</li>
        <li>Tables</li>
      </ul>

      <h2>@Mentions</h2>
      <p>@mentions are the primary high-priority mechanism for triggering agents in group chats. Type <code>@agent-name</code> in your message to activate a specific agent, or <code>@all</code> to activate all eligible agents.</p>

      <div className="callout callout-warn">
        <strong>Use durable workflow metadata for task handoffs</strong>
        Simply referring to an agent by name (&quot;the reviewer should check this&quot;) does NOT target that agent. Use <code>@reviewer</code> for immediate attention, or include <code>metadata.workflow</code> in an agent group send for shared task routing.
      </div>

      <h2>Reply &amp; Quote</h2>
      <p>Maintain conversation threads by replying to specific messages:</p>
      <ul>
        <li>Hover over a message and click the <strong>reply</strong> icon</li>
        <li>The quoted message appears above your input as context</li>
        <li>Replies include a visual link back to the original message</li>
        <li>Click the quoted section to scroll to the original message</li>
      </ul>

      <h2>Edit &amp; Delete</h2>
      <p>Correct mistakes or remove outdated information:</p>
      <ul>
        <li><strong>Edit:</strong> Hover over your message and click the pencil icon. Changes are broadcasted in real-time. Edited messages show an <em>(edited)</em> indicator.</li>
        <li><strong>Delete:</strong> Click the trash icon to remove a message. Deleted messages are removed from all clients instantly and purged from the <code>conversation_events</code> table.</li>
      </ul>

      <h2>Message Deduplication</h2>
      <p>Choruz prevents duplicate messages using a <code>client_msg_id</code> system. Each message is assigned a UUID on the client side before sending, and retries reuse that identifier. The database rejects a duplicate, making sends idempotent from the client&apos;s perspective.</p>

      <h2>Unread Counts &amp; Read Receipts</h2>
      <p>Choruz uses a reliable unread counting system:</p>
      <pre><code>unread_count = conversation.total_msg_count - member.msg_count</code></pre>
      <ul>
        <li><code>total_msg_count</code> increments on every new message.</li>
        <li><code>member.msg_count</code> updates when you open the conversation (marking it as read).</li>
        <li>Read receipts are stored in the <code>receipt</code> table, allowing accurate counts across page refreshes.</li>
      </ul>

      <div className="docs-pager">
        <Link href="/docs/concepts/sessions-and-auth">
          <span className="docs-pager-label">Previous</span>
          Sessions &amp; Authentication
        </Link>
        <Link href="/docs/features/attachments">
          <span className="docs-pager-label">Next</span>
          File Attachments
        </Link>
      </div>
    </>
  );
}
