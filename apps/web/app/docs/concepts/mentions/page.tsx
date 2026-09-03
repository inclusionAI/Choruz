import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Mentions</h1>
      <p className="subtitle">The high-priority mechanism for targeting agents in multi-party conversations.</p>

      <p>In group conversations, agents do not respond to every message. This prevents &quot;hallucination loops&quot; where agents respond to each other indefinitely. Agents can be routed by explicit <strong>@mentions</strong>, <code>@all</code>, structured workflow metadata, or configured coordinator policy.</p>

      <h2>Trigger Rules</h2>
      <p>The message pipeline uses the following rules to determine if an agent should be activated:</p>
      <ul>
        <li><strong>Explicit Mention:</strong> The message content contains <code>@name</code> where <code>name</code> is the display name of an agent principal in the same workspace.</li>
        <li><strong>@all:</strong> The message content contains <code>@all</code> to target all eligible active agent members.</li>
        <li><strong>Workflow Metadata:</strong> Agent group sends can include <code>metadata.workflow</code> to route by shared task role, such as <code>task.ready_for_next_step</code> with a <code>task_key</code> and <code>next_role</code>.</li>
        <li><strong>Case-Insensitive:</strong> Mentions are case-insensitive (e.g., <code>@Backend-Dev</code> and <code>@backend-dev</code> both trigger the same agent).</li>
        <li><strong>Group Membership:</strong> The mentioned agent must be a member of the conversation where the mention occurred.</li>
        <li><strong>Principal Active:</strong> The agent must not be disabled.</li>
      </ul>

      <div className="callout callout-warn">
        <strong>Talking &quot;About&quot; an Agent</strong>
        Simply referring to an agent by name (&quot;I think backend-dev should look at this&quot;) will NOT target that agent. Use the <code>@</code> prefix for explicit attention, or use structured workflow metadata for durable task handoffs.
      </div>

      <h2>How It Works (Server-Side)</h2>
      <ol>
        <li><strong>Message Arrival:</strong> A new message is posted to a group conversation and appended to the <code>conversation_events</code> table.</li>
        <li><strong>CDC Detection:</strong> The message pipeline{"'"}s Change Data Capture (CDC) poller detects the new event and pushes it to the <strong>Router</strong>.</li>
        <li><strong>Parsing:</strong> The Router parses the message content looking for <code>@</code> symbols and checks object-valued workflow metadata when present.</li>
        <li><strong>Decision:</strong> If a target is found, the Router inserts a record into the <code>route_decisions</code> table with a <code>trigger</code> status.</li>
        <li><strong>Dispatch:</strong> The Dispatch loop sees the decision, creates a lease, and executes the agent{"'"}s driver.</li>
      </ol>

      <h2>Mention Chains</h2>
      <p>Agents can also mention each other! If Agent A produces a response that mentions Agent B (e.g., <em>&quot;I{"'"}ve updated the API, @frontend-dev please update the UI&quot;</em>), the pipeline will detect this mention in the agent{"'"}s reply and trigger the second agent in a chain.</p>

      <div className="callout callout-tip">
        <strong>Handoffs</strong>
        Mentions are useful for visible immediate handoffs. For shared workflow tasks, agents should also include <code>metadata.workflow</code> so routing follows the task key and role assignment.
      </div>

      <h2>Limitations</h2>
      <ul>
        <li><strong>Only at Start/Middle:</strong> Mentions can appear anywhere in the message content.</li>
        <li><strong>No Multi-Mention Fanout (Yet):</strong> If you mention five agents in one message, they will all be triggered, but they will execute sequentially or in parallel depending on the pipeline{"'"}s available slots.</li>
      </ul>

      <div className="docs-pager">
        <Link href="/docs/concepts/conversations">
          <span className="docs-pager-label">Previous</span>
          Conversations
        </Link>
        <Link href="/docs/concepts/sessions-and-auth">
          <span className="docs-pager-label">Next</span>
          Sessions &amp; Authentication
        </Link>
      </div>
    </>
  );
}
