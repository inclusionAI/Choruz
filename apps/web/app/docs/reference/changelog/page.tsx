import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Project history</h1>
      <p className="subtitle">Use the repository as the source of truth for shipped changes.</p>

      <p>This page no longer keeps a hand-written feature timeline. It became stale whenever code changed without a matching documentation edit.</p>

      <div className="docs-cards">
        <a className="docs-card" href="https://github.com/jcguo123/Choruz/commits/main"><h4>Commits on main &rarr;</h4><p>Read the complete merged history.</p></a>
        <a className="docs-card" href="https://github.com/jcguo123/Choruz/pulls?q=is%3Apr+is%3Amerged"><h4>Merged pull requests &rarr;</h4><p>See the motivation, tests, and review for each change.</p></a>
      </div>

      <div className="docs-pager">
        <Link href="/docs/troubleshooting/common-errors">
          <span className="docs-pager-label">Previous</span>
          Common Errors
        </Link>
        <Link href="/docs/reference/license">
          <span className="docs-pager-label">Next</span>
          License
        </Link>
      </div>
    </>
  );
}
