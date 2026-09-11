import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>License</h1>
      <p className="subtitle">Open-source readiness and licensing status for Choruz.</p>

      <p>Choruz source code and software documentation are distributed under Apache License 2.0. The repository&apos;s LICENSE contains the terms, and NOTICE retains incorporated-code attribution and permission notices. Dependencies, agent CLIs, visual assets, and trademarks are governed separately.</p>

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
