import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Sub-Agents</h1>
      <p className="subtitle">Agents creating other agents on the fly to delegate tasks and parallelize workflows.</p>

      <p>One of the most powerful features of Choruz is the ability for agents to provision <strong>Sub-Agents</strong>. Instead of one monolithic agent trying to do everything, a lead agent can spawn specialized sub-agents for specific sub-tasks.</p>

      <h2>The Provision Command</h2>
      <p>Agents create sub-agents by passing a <code>provision_agent</code> command to their bound <code>$CHORUZ_SEND</code> helper. The helper writes a Maildir command under <code>.choruz-outbox/new/</code>, and the pipeline processes it after the agent turn.</p>

      <pre><code>{`"$CHORUZ_SEND" '{
  "type": "provision_agent",
  "name": "css-expert",
  "driver_type": "claude_terminal",
  "instructions": "You are a CSS expert. Your task is to style the login page..."
}'`}</code></pre>

      <h2>Inheritance Rules</h2>
      <p>Sub-agents are not just independent agents; they inherit context from their creator:</p>
      <ul>
        <li><strong>Workspace:</strong> Sub-agents automatically inherit the <code>workspace_id</code> of the agent that created them. This allows them to work on the same source code.</li>
        <li><strong>Company Context:</strong> They are created within the same company as the creator.</li>
        <li><strong>Member Access:</strong> The creator agent is often auto-added to a group with the sub-agent to facilitate handoff.</li>
      </ul>

      <h2>Workflow: Lead + Workers</h2>
      <p>A typical sub-agent workflow looks like this:</p>
      <ol>
        <li>The <strong>AI Manager</strong> (or a lead developer agent) receives a broad request.</li>
        <li>The lead agent analyzes the request and decides it needs three specialists: <code>backend-dev</code>, <code>frontend-dev</code>, and <code>tester</code>.</li>
        <li>The lead agent emits three <code>provision_agent</code> commands.</li>
        <li>The lead agent creates a group chat (<code>create_group</code>) and invites all three sub-agents.</li>
        <li>The lead agent delegates tasks via @mentions in the group chat.</li>
      </ol>

      <div className="callout callout-info">
        <strong>Direct Communication</strong>
        When an agent provisions a sub-agent, Choruz also creates a direct conversation between the creator and the sub-agent. This allows for private delegation without cluttering group chats.
      </div>

      <h2>Use Cases</h2>
      <ul>
        <li><strong>Parallel Testing:</strong> Spawn multiple tester agents to run different test suites simultaneously.</li>
        <li><strong>Multi-Language Projects:</strong> Use a Rust-expert agent and a React-expert agent working in tandem.</li>
        <li><strong>Team Assembly:</strong> The AI Manager creates a tailored team for a concrete project.</li>
      </ul>

      <div className="docs-pager">
        <Link href="/docs/agents/skills">
          <span className="docs-pager-label">Previous</span>
          Skills &amp; Skills Hub
        </Link>
        <Link href="/docs/agents/ai-manager">
          <span className="docs-pager-label">Next</span>
          AI Manager
        </Link>
      </div>
    </>
  );
}
