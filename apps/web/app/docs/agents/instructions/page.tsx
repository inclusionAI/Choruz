import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Writing Instructions</h1>
      <p className="subtitle">Agent instructions are written in CLAUDE.md or AGENTS.md and define the agent{"'"}s identity, behavior, and capabilities.</p>

      <h2>Instruction Files</h2>
      <p>Each CLI driver reads its own instruction file from the agent{"'"}s workspace root:</p>
      <table>
        <thead><tr><th>Driver</th><th>Instruction File</th></tr></thead>
        <tbody>
          <tr><td><code>claude_terminal</code> / <code>claude_print</code></td><td><code>CLAUDE.md</code></td></tr>
          <tr><td><code>codex_terminal</code> / <code>codex_exec</code> / <code>pi_terminal</code> / <code>grok_terminal</code> / <code>opencode_terminal</code></td><td><code>AGENTS.md</code></td></tr>
        </tbody>
      </table>

      <h2>How Instructions Are Composed</h2>
      <p>When Choruz provisions an agent, it writes the instruction file by combining two parts:</p>

      <div className="docs-steps">
        <div className="docs-step">
          <div className="docs-step-num">1</div>
          <div>
            <h4>Platform Instructions (auto-injected)</h4>
            <p>Choruz automatically injects the platform protocol documentation into every agent{"'"}s instruction file. This teaches agents how to use the outbox protocol, respond to group chats, @mention other agents, share files, create sub-agents, and coordinate with teams. You never need to write this yourself &mdash; it is appended automatically.</p>
          </div>
        </div>
        <div className="docs-step">
          <div className="docs-step-num">2</div>
          <div>
            <h4>Custom Instructions (user-provided)</h4>
            <p>Your custom instructions &mdash; the agent{"'"}s role, goals, SOP, and constraints &mdash; are prepended before the platform instructions. This is the part you write (or the AI Manager writes for you).</p>
          </div>
        </div>
      </div>

      <p>The final instruction file looks like:</p>
      <pre><code>{`# [Your Custom Instructions]
## Role
You are a backend developer specializing in Rust...

## Project Context
...

# [Choruz Platform Instructions - auto-injected]
## Who You Are
You're an AI agent running on Choruz...

## How Choruz Works
...`}</code></pre>

      <div className="callout callout-info">
        <strong>You only write the custom part</strong>
        The Choruz platform instructions (outbox protocol, group chat rules, team coordination) are injected automatically. Focus your instructions on what makes this specific agent unique.
      </div>

      <h2>The Five Sections</h2>
      <p>The instruction editor and the AI Manager use the same five sections. Each label in the editor has an info icon that explains what belongs there:</p>

      <table>
        <thead><tr><th>#</th><th>Section</th><th>Purpose</th><th>Example</th></tr></thead>
        <tbody>
          <tr><td>1</td><td><strong>Role</strong></td><td>Who the agent is and what it owns: expertise, backstory, goals</td><td>&quot;You are a senior backend engineer specializing in Rust and PostgreSQL. You own the REST API and its test coverage&quot;</td></tr>
          <tr><td>2</td><td><strong>Project Context</strong></td><td>Tech stack, key files, workspace paths</td><td>&quot;Stack: Rust + Axum + PostgreSQL. Main entry: src/main.rs&quot;</td></tr>
          <tr><td>3</td><td><strong>Boundaries</strong></td><td>What it may do, what it must never do, and how its output should look</td><td>&quot;May run cargo build and cargo test. Never push to main. Keep group replies under 5 lines&quot;</td></tr>
          <tr><td>4</td><td><strong>Workflow</strong></td><td>Step-by-step process, what counts as done, what to do on failure</td><td>&quot;1. Read the task 2. Write tests 3. Implement 4. Report. Done when tests pass; if the environment is broken, report [BLOCKED]&quot;</td></tr>
          <tr><td>5</td><td><strong>Collaboration</strong></td><td>Who it talks to, what triggers it, when to escalate</td><td>&quot;Respond to @mentions from the reviewer. If blocked for more than 3 attempts, @mention the leader&quot;</td></tr>
        </tbody>
      </table>
      <p>The <code>[DONE]</code> / <code>[BLOCKED]</code> report format, <code>@mention</code> triggers and the task board commands are part of the platform protocol and do not need repeating. Periodic work is scheduled through the agent&apos;s Cron settings, not through prose.</p>

      <h2>Example: Custom Instructions</h2>
      <pre><code>{`## Role
You are a test engineer specializing in integration and end-to-end testing.
You have deep expertise in Playwright, Jest, and Rust test frameworks.
- Write comprehensive tests for all new features
- Maintain test coverage above 80%
- Run tests before any PR merge

## Project Context
- Language: TypeScript (frontend), Rust (backend)
- Test framework: Playwright for E2E, Jest for unit tests
- Test directory: tests/
- Config: playwright.config.ts

## Boundaries
- Run: npm test, npx playwright test, cargo test
- Edit files in: tests/, src/__tests__/
- Never modify production source code
- Never mark tests as .skip without leader approval
- Be thorough but concise in reports; include exact error messages

## Workflow
1. Read the feature requirements or PR description
2. Write test cases covering happy path, edge cases, and error cases
3. Run the tests locally
4. Report results with pass/fail counts
Done when all tests pass, edge cases are covered, and no test is flaky.
Retry flaky tests up to 3 times; if the environment is broken, report it immediately.

## Collaboration
- Share test file paths when done
- @mention the developer if tests reveal bugs
- If a feature is untestable or the environment is broken, @mention the leader`}</code></pre>

      <h2>Editing Instructions via the UI</h2>
      <p>You can edit an agent{"'"}s instructions at any time through the Detail Panel:</p>
      <ol>
        <li>Click on the agent{"'"}s conversation in the sidebar</li>
        <li>Open the Detail Panel (click the panel toggle or drag the right edge)</li>
        <li>Switch to the <strong>Config</strong> tab</li>
        <li>Edit the instruction text and click <strong>Save</strong></li>
      </ol>
      <p>Changes take effect on the agent{"'"}s next activation &mdash; the instruction file is rewritten in the workspace.</p>

      <h2>Programmatic Instruction Updates</h2>
      <p>Agent creation flows accept an <code>instructions</code> field when provisioning a new agent. For existing agents, use the UI editor so the workspace instruction file is rewritten consistently with the selected driver.</p>

      <div className="callout callout-tip">
        <strong>Let the AI Manager write instructions</strong>
        For best results, describe what you want in plain English to the AI Manager. It will generate complete five-section instructions based on your description, tailored to the specific CLI driver and team context.
      </div>

      <h2>Best Practices</h2>
      <ul>
        <li><strong>Be specific.</strong> Vague instructions like &quot;write good code&quot; produce vague behavior. Include file paths, command names, and exact formats.</li>
        <li><strong>Define boundaries.</strong> The Boundaries section is where you stop an agent from overstepping; write the forbidden part first.</li>
        <li><strong>Include examples.</strong> Show the exact format of status reports, commit messages, or test output you expect.</li>
        <li><strong>Keep it structured.</strong> Agents parse markdown headings &mdash; use clear section headers.</li>
        <li><strong>Test and iterate.</strong> Watch the agent work, then refine instructions based on what it gets wrong.</li>
      </ul>

      <div className="docs-pager">
        <Link href="/docs/agents/drivers">
          <span className="docs-pager-label">Previous</span>
          Agent Drivers
        </Link>
        <Link href="/docs/agents/skills">
          <span className="docs-pager-label">Next</span>
          Skills &amp; Skills Hub
        </Link>
      </div>
    </>
  );
}
