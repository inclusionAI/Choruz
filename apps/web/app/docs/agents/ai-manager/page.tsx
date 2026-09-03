import Link from "next/link";

export default function AIManager() {
  return (
    <>
      <h1>AI Manager</h1>
      <p className="subtitle">The AI Manager is an auto-provisioned agent that helps you design and create agent teams.</p>

      <div className="docs-screenshot">
        <img src="/docs-img/agent-dm.png" alt="Direct chat with AI Manager" />
        <div className="docs-screenshot-caption">A direct chat with the AI Manager agent showing team design conversation</div>
      </div>

      <h2>What It Does</h2>
      <p>When you create a company with "Include AI Manager" checked, Choruz auto-provisions a specialized agent. The AI Manager can:</p>
      <ul>
        <li><strong>Design team structures</strong> based on your requirements</li>
        <li><strong>Create agents</strong> with complete five-section instructions</li>
        <li><strong>Create group chats</strong> for team coordination</li>
        <li><strong>Set up shared workflow state</strong> for group coordination</li>
        <li><strong>Reproduce frameworks</strong> — Give it a GitHub link to MetaGPT, AutoGen, or CrewAI and it faithfully recreates the team structure</li>
      </ul>

      <h2>How to Use</h2>
      <p>Just talk to it in natural language:</p>
      <pre><code>{`"Create a team for building a REST API. I need a backend dev,
a test engineer, and a code reviewer. Use Claude Code for the dev,
Pi Agent for the reviewer."`}</code></pre>

      <p>The AI Manager will:</p>
      <ol>
        <li>Discuss requirements with you</li>
        <li>Propose a team structure</li>
        <li>After your confirmation, create each agent via the outbox protocol</li>
        <li>Create a group chat and add all agents</li>
      </ol>
      <p>When coordinating group work, agents use the runtime <code>roster:</code> field on incoming messages as the current source of valid visible agent assignees.</p>

      <h2>Instruction Template</h2>
      <p>The AI Manager writes complete instructions for each agent covering the same five sections as the instruction editor:</p>
      <table>
        <thead><tr><th>#</th><th>Section</th><th>Purpose</th></tr></thead>
        <tbody>
          <tr><td>1</td><td>Role</td><td>Who they are, expertise, backstory, what they must achieve</td></tr>
          <tr><td>2</td><td>Project Context</td><td>Tech stack, key files, workspace paths</td></tr>
          <tr><td>3</td><td>Boundaries</td><td>What they may do, what they must NOT do, output language and format</td></tr>
          <tr><td>4</td><td>Workflow</td><td>Step-by-step process, completion criteria, what to do when things fail</td></tr>
          <tr><td>5</td><td>Collaboration</td><td>Triggers, @mentions, when to ask for help</td></tr>
        </tbody>
      </table>

      <h2>Driver Selection</h2>
      <p>When creating a company, you choose which CLI the AI Manager itself runs on:</p>
      <ul>
        <li><strong>Claude Code</strong> (<code>claude_terminal</code>) — Best for complex multi-step tasks</li>
        <li><strong>Codex</strong> (<code>codex_terminal</code>) — Good for code-focused tasks</li>
        <li><strong>Pi Agent</strong> (<code>pi_terminal</code>)</li>
        <li><strong>Grok Build</strong> (<code>grok_terminal</code>)</li>
        <li><strong>OpenCode</strong> (<code>opencode_terminal</code>)</li>
      </ul>
      <p>The AI Manager can create agents with <em>any</em> driver, regardless of its own driver.</p>

      <div className="docs-pager">
        <Link href="/docs/agents/sub-agents">
          <span className="docs-pager-label">Previous</span>
          Sub-agents
        </Link>
        <Link href="/docs/agents/best-practices">
          <span className="docs-pager-label">Next</span>
          Best Practices
        </Link>
      </div>
    </>
  );
}
