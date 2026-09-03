import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Agent Skills</h1>
      <p className="subtitle">Extend agent capabilities with reusable skills stored in workspace directories, shared through a skills hub, and automatically provisioned.</p>

      <h2>Overview</h2>
      <p>Skills are reusable instruction sets and commands that extend what an agent can do. They live as files in the agent{"'"}s workspace and are automatically discovered by the CLI at runtime. Skills allow you to give agents specialized capabilities without modifying their core instructions.</p>

      <p>The management and provisioning UI is supplied by the <code>agent-skills</code> plugin. Excluding it from <code>CHORUZ_PLUGINS</code> removes Skills tabs and provisioning controls and disables the Web skill-management APIs.</p>

      <h2>Skill Directories</h2>
      <p>Agents discover skills from two directories in their workspace:</p>

      <table>
        <thead><tr><th>Directory</th><th>Purpose</th><th>Format</th></tr></thead>
        <tbody>
          <tr><td><code>.claude/skills/</code></td><td>Skill definitions with metadata and instructions</td><td>Markdown files with frontmatter</td></tr>
          <tr><td><code>.claude/commands/</code></td><td>Slash commands that agents can execute</td><td>Markdown files defining command behavior</td></tr>
        </tbody>
      </table>

      <h3>.claude/skills/</h3>
      <p>Skills in this directory provide specialized knowledge and behaviors. Each skill is a markdown file:</p>

      <pre><code>{`.claude/skills/
  code-review.md
  test-writer.md
  security-audit.md`}</code></pre>

      <p>A skill file contains instructions that the CLI loads as additional context for the agent. For example, a <code>code-review.md</code> skill might contain patterns for reviewing code, common issues to check for, and output format guidelines.</p>

      <h3>.claude/commands/</h3>
      <p>Commands in this directory define slash commands the agent can use:</p>

      <pre><code>{`.claude/commands/
  deploy.md
  run-tests.md
  lint.md`}</code></pre>

      <p>When an agent (or user) types <code>/deploy</code>, the CLI loads <code>.claude/commands/deploy.md</code> and executes the instructions within.</p>

      <h2>Skill Scanning</h2>
      <p>The frontend provides a skill scanning API that discovers available skills in an agent{"'"}s workspace:</p>

      <pre><code>{`GET /api/agent-skills?workspace=/path/to/agent/workspace`}</code></pre>

      <p>This endpoint:</p>
      <ul>
        <li>Scans both <code>.claude/skills/</code> and <code>.claude/commands/</code> directories</li>
        <li>Returns a list of discovered skill files with their names and paths</li>
        <li>Is used by the detail panel to show available skills for each agent</li>
      </ul>

      <h2>Skills Hub</h2>
      <p>The Skills Hub is a central repository of pre-built skills that can be browsed and installed into agent workspaces. It provides:</p>
      <ul>
        <li><strong>Skill catalog</strong> &mdash; Browse available skills by category</li>
        <li><strong>One-click install</strong> &mdash; Copy a skill into an agent{"'"}s workspace</li>
        <li><strong>External skills</strong> &mdash; Import skills from external sources</li>
      </ul>

      <h2>Provisioning with Skills</h2>
      <p>When creating an agent, you can specify skill paths to be copied into the agent{"'"}s workspace during provisioning:</p>

      <pre><code>{`POST /api/agents/provision
Content-Type: application/json

{
  "name": "code-reviewer",
  "driver_type": "claude_terminal",
  "workspace_id": "<company-id>",
  "instructions": "You are a code reviewer.",
  "skill_paths": [
    "/Users/alice/skills/code-review.md",
    "/Users/alice/skills/security-audit.md"
  ]
}`}</code></pre>

      <p>The <code>skill_paths</code> parameter accepts absolute paths inside the server user{"'"}s <code>$HOME</code>; replace <code>/Users/alice</code> with that home directory. Provisioning copies those files into the new agent{"'"}s <code>.claude/skills/</code> directory before the agent starts.</p>

      <div className="callout callout-info">
        <strong>Skill inheritance</strong>
        When the AI Manager creates a team of agents, it can assign different skills to each agent based on their role. A test engineer gets testing skills, a reviewer gets review skills, and so on. This specializes each agent{"'"}s behavior without changing their core instructions.
      </div>

      <h2>Creating Custom Skills</h2>
      <p>To create a new skill, add a markdown file to the appropriate directory:</p>

      <div className="docs-steps">
        <div className="docs-step">
          <div className="docs-step-num">1</div>
          <div>
            <h4>Choose the directory</h4>
            <p>Use <code>.claude/skills/</code> for knowledge/behavior extensions, or <code>.claude/commands/</code> for slash commands.</p>
          </div>
        </div>
        <div className="docs-step">
          <div className="docs-step-num">2</div>
          <div>
            <h4>Write the skill file</h4>
            <p>Create a markdown file with clear instructions. Include context about when the skill should be used, what it does, and any constraints.</p>
          </div>
        </div>
        <div className="docs-step">
          <div className="docs-step-num">3</div>
          <div>
            <h4>Test the skill</h4>
            <p>Provision an agent with the skill and verify it behaves as expected. Check that the skill appears in the agent{"'"}s detail panel.</p>
          </div>
        </div>
      </div>

      <div className="callout callout-tip">
        <strong>Skill best practices</strong>
        Keep skills focused on a single capability. A skill that tries to do too much becomes hard to maintain and may conflict with other skills. Write skills as if you are writing instructions for a new team member &mdash; clear, specific, and actionable.
      </div>

      <div className="docs-pager">
        <Link href="/docs/agents/instructions">
          <span className="docs-pager-label">Previous</span>
          Writing Instructions
        </Link>
        <Link href="/docs/agents/sub-agents">
          <span className="docs-pager-label">Next</span>
          Sub-agents
        </Link>
      </div>
    </>
  );
}
