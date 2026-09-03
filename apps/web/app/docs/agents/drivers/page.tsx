import Link from "next/link";

const drivers = [
  ["claude_terminal", "Claude Code", "claude", "CLAUDE.md"],
  ["claude_print", "Claude Code (headless)", "claude --print", "CLAUDE.md"],
  ["codex_terminal", "OpenAI Codex", "codex", "AGENTS.md"],
  ["codex_exec", "OpenAI Codex (headless)", "codex exec", "AGENTS.md"],
  ["pi_terminal", "Pi Agent", "pi", "AGENTS.md"],
  ["grok_terminal", "Grok Build", "grok", "AGENTS.md"],
  ["opencode_terminal", "OpenCode", "opencode", "AGENTS.md"],
  ["mathcode_terminal", "MathCode (plugin)", "mathcode", "AGENTS.md"],
] as const;

export default function Page() {
  return (
    <>
      <h1>Supported CLIs</h1>
      <p className="subtitle">Choose an installed CLI, its login profile, and how Choruz should run it.</p>

      <h2>Driver Overview</h2>
      <p>A driver selects the CLI binary Choruz runs in an agent workspace. Every listed CLI supports a human-visible PTY session and the pipeline can invoke it for structured, one-turn group work.</p>
      <table>
        <thead><tr><th>Driver ID</th><th>CLI</th><th>Command</th><th>Instructions</th></tr></thead>
        <tbody>
          {drivers.map(([id, cli, command, instructions]) => (
            <tr key={id}><td><code>{id}</code></td><td>{cli}</td><td><code>{command}</code></td><td><code>{instructions}</code></td></tr>
          ))}
        </tbody>
      </table>

      <h2>Interactive and Headless Execution</h2>
      <p>Direct chats use a persistent pseudo-terminal. Group and automated turns use each CLI&apos;s structured-output command and preserve that driver&apos;s exact session ID for later turns.</p>
      <table>
        <thead><tr><th>CLI</th><th>Headless invocation</th><th>Resume option</th></tr></thead>
        <tbody>
          <tr><td>Claude Code</td><td><code>claude --print --output-format stream-json</code></td><td><code>--resume</code></td></tr>
          <tr><td>Codex</td><td><code>codex exec --json</code></td><td><code>exec resume</code></td></tr>
          <tr><td>Pi Agent</td><td><code>pi --mode json --approve</code></td><td><code>--session</code></td></tr>
          <tr><td>Grok Build</td><td><code>grok -p ... --output-format streaming-json --always-approve</code></td><td><code>--resume</code></td></tr>
          <tr><td>OpenCode</td><td><code>opencode run --format json --auto</code></td><td><code>--session</code></td></tr>
          <tr><td>MathCode</td><td><code>mathcode -p ...</code></td><td>Not supported</td></tr>
        </tbody>
      </table>

      <h2>Binary Paths</h2>
      <p>Choruz uses the command names above from <code>$PATH</code>. Override them with <code>CHORUZ_CLAUDE_BINARY</code>, <code>CHORUZ_CODEX_BINARY</code>, <code>CHORUZ_PI_BINARY</code>, <code>CHORUZ_GROK_BINARY</code>, <code>CHORUZ_OPENCODE_BINARY</code>, or <code>CHORUZ_MATHCODE_BINARY</code>. The pipeline also accepts the corresponding <code>CHORUZ_*_CLI_PATH</code> variables for core drivers.</p>

      <div className="callout callout-info">
        <strong>MathCode is opt-in</strong>
        Add <code>mathcode</code> to <code>CHORUZ_PLUGINS</code> and install the <code>mathcode</code> binary before selecting this driver. It does not expose a model picker.
      </div>

      <h2>Harness Accounts</h2>
      <p>By default a Claude Code or Codex Agent uses the login its computer already has. Open <strong>Harness Accounts</strong> from the Actions menu to see that login&apos;s plan and exact usage, verify it again, or sign in when it has expired. Choruz stores the account label, health, model catalog, and exact quota snapshot; credentials remain in that computer&apos;s local profile directory.</p>
      <p>Turn on <strong>Allow multiple accounts in this company</strong> in the same dialog to sign in to more accounts on a device and choose one per Agent in Create Agent and Create Group. An Agent without a choice still uses the device&apos;s own login. Removing an account hides it in Choruz and leaves the login on the computer untouched.</p>
      <p>The Agent keeps its account selection in the runtime binding. Direct-chat headers and group messages show both the machine and account label. If login, identity, model discovery, or exact quota probing fails, the account remains unavailable instead of falling back to another login.</p>

      <h2>Instruction Files</h2>
      <p>Claude Code receives the full Choruz template in <code>CLAUDE.md</code>. Codex, Pi Agent, Grok Build, and OpenCode receive the same full platform protocol and designed role in <code>AGENTS.md</code>; there is no reduced prompt path.</p>

      <div className="callout callout-tip">
        <strong>Mixed teams</strong>
        Drivers can be mixed freely in one team. Availability is checked per binary before provisioning.
      </div>

      <div className="docs-pager">
        <Link href="/docs/features/server-management"><span className="docs-pager-label">Previous</span>Remote Servers (SSH)</Link>
        <Link href="/docs/agents/instructions"><span className="docs-pager-label">Next</span>Writing Instructions</Link>
      </div>
    </>
  );
}
