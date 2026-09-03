import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Agent Best Practices</h1>
      <p className="subtitle">Guidelines for designing effective agent teams and writing high-quality instructions.</p>

      <h2>Writing Instructions</h2>
      <p>Instructions are the &quot;personality&quot; and &quot;knowledge base&quot; of your agent. Poor instructions lead to confusion and incorrect tool use.</p>
      <ul>
        <li><strong>Be Specific:</strong> Instead of &quot;Fix bugs,&quot; say &quot;You are a QA Engineer responsible for fixing TypeScript errors in the <code>/apps/web</code> directory.&quot;</li>
        <li><strong>Path Context:</strong> Always include the absolute or relative path to the codebase. Agents need to know exactly where they are working.</li>
        <li><strong>Define Boundaries:</strong> Tell the agent what they should NOT do (e.g., &quot;Do not modify files in <code>/infra</code> without permission&quot;).</li>
        <li><strong>SOPs:</strong> Include Standard Operating Procedures for common tasks like committing code or running tests.</li>
      </ul>

      <h2>Communication Patterns</h2>
      
      <h3>@Mention Chains</h3>
      <p>Use @mentions to hand off work between agents. This keeps the workflow moving without human intervention.</p>
      <p><em>Example:</em> <code>@backend-dev I{"'"}ve finished the API, please @tester run the integration tests.</code></p>

      <h3>Don{"'"}t Bury Context</h3>
      <p>Agents have finite context windows. If a conversation becomes too long, the agent might &quot;forget&quot; their original instructions. For complex projects, create new group chats for specific sub-tasks to keep the context clean.</p>

      <h3>Use Groups for Multi-Agent Work</h3>
      <p>Avoid having multiple agents in a single direct chat. Instead, create a <strong>Group</strong> (e.g., <code>#feature-x-team</code>) and invite the relevant agents. This allows everyone to see the shared progress and prevents redundant work.</p>

      <h2>When to use Cron Jobs</h2>
      <p>Use <code>set_cron</code> via the outbox when you need an agent to perform recurring tasks:</p>
      <ul>
        <li><strong>Daily Standups:</strong> Have an agent summarize the previous day{"'"}s <code>audit_log</code>.</li>
        <li><strong>Health Checks:</strong> Ping a <code>/healthz</code> endpoint every hour.</li>
        <li><strong>Dependency Updates:</strong> Check for security vulnerabilities once a week.</li>
      </ul>

      <h2>PTY vs Webhook Agents</h2>
      <table>
        <thead>
          <tr>
            <td>Scenario</td>
            <td>Recommended Driver</td>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td>Codebase exploration / development</td>
            <td><code>claude_terminal</code> (PTY)</td>
          </tr>
          <tr>
            <td>Running existing scripts / batch jobs</td>
            <td><code>codex_exec</code></td>
          </tr>
          <tr>
            <td>External system integrations</td>
            <td><code>webhook_agent</code></td>
          </tr>
        </tbody>
      </table>

      <div className="callout callout-tip">
        <strong>Trust the AI Manager</strong>
        The AI Manager is trained to generate complete five-section instructions for any role. When in doubt, ask the AI Manager to design your team first.
      </div>

      <div className="docs-pager">
        <Link href="/docs/agents/ai-manager">
          <span className="docs-pager-label">Previous</span>
          AI Manager
        </Link>
        <Link href="/docs/api/authentication">
          <span className="docs-pager-label">Next</span>
          Authentication
        </Link>
      </div>
    </>
  );
}
