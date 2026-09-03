import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>License</h1>
      <p className="subtitle">Open-source readiness and licensing status for Choruz.</p>

      <p>Choruz does not currently include a root <code>LICENSE</code> file. Do not assume permission to use, redistribute, or contribute until the repository owner selects and adds one. See the repository&apos;s open-source-readiness checklist for the remaining launch requirements.</p>

      <h2>Third-Party Licenses</h2>
      <p>Choruz is built on top of several open-source libraries and frameworks, including:</p>
      <ul>
        <li><strong>Rust:</strong> Apache License 2.0 / MIT.</li>
        <li><strong>Next.js:</strong> MIT License.</li>
        <li><strong>PostgreSQL:</strong> PostgreSQL License.</li>
        <li><strong>CodeMirror:</strong> MIT License.</li>
        <li><strong>Agent CLIs:</strong> Claude Code, Codex, Pi Agent, Grok Build, and OpenCode remain subject to their respective licenses.</li>
      </ul>

      <div className="callout callout-info">
        <strong>Commercial Use</strong>
        For commercial support, private hosting, or custom driver development, please contact the maintainers via the GitHub repository.
      </div>

      <div className="docs-pager">
        <Link href="/docs/reference/changelog">
          <span className="docs-pager-label">Previous</span>
          Changelog
        </Link>
        <div />
      </div>
    </>
  );
}
