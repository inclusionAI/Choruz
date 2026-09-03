import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Search</h1>
      <p className="subtitle">Full-text search across all messages with trigram-based fuzzy matching and instant navigation to results.</p>

      <div className="docs-screenshot">
        <img src="/docs-img/search.png" alt="Search results" />
        <div className="docs-screenshot-caption">Search results appearing in the sidebar with matched messages and conversation context</div>
      </div>

      <h2>Overview</h2>
      <p>Choruz provides full-text search across all messages in the system. The search uses PostgreSQL{"'"}s trigram indexing for fast fuzzy matching, meaning you can find messages even with partial words or slight misspellings.</p>

      <h2>Using Search</h2>

      <h3>Search Bar</h3>
      <p>The search bar is located in the sidebar. Type your query and results appear in real-time as you type.</p>
      <ul>
        <li>Results show the matching message text with context</li>
        <li>Each result shows which conversation the message belongs to</li>
        <li>Click a result to jump directly to that message in the conversation</li>
        <li>The conversation scrolls to the matched message and highlights it</li>
      </ul>

      <h2>How It Works</h2>

      <h3>Trigram Index</h3>
      <p>Choruz uses PostgreSQL{"'"}s <code>pg_trgm</code> extension to create a trigram index on message content. A trigram is a group of three consecutive characters. For example, the word &quot;hello&quot; produces the trigrams: &quot;hel&quot;, &quot;ell&quot;, &quot;llo&quot;.</p>
      <ul>
        <li><strong>Fuzzy matching</strong> &mdash; Finds results even when the query has typos or partial words</li>
        <li><strong>Case-insensitive</strong> &mdash; Searches are case-insensitive by default</li>
        <li><strong>Fast</strong> &mdash; The trigram index (GIN index) makes searches fast even across millions of messages</li>
      </ul>

      <pre><code>{`-- The trigram index on message content
CREATE INDEX idx_messages_content_trgm
  ON conversation_events
  USING gin (content gin_trgm_ops);`}</code></pre>

      <h3>Search Query</h3>
      <p>The search endpoint uses trigram similarity to rank results:</p>
      <pre><code>{`SELECT id, content, conversation_id, created_at,
       similarity(content, $1) AS rank
FROM conversation_events
WHERE content % $1
ORDER BY rank DESC
LIMIT 50;`}</code></pre>

      <h2>API Endpoint</h2>
      <pre><code>{`GET /v1/messages/search?q=search+term`}</code></pre>

      <h3>Parameters</h3>
      <table>
        <thead><tr><th>Parameter</th><th>Required</th><th>Description</th></tr></thead>
        <tbody>
          <tr><td><code>q</code></td><td>Yes</td><td>The search query string</td></tr>
        </tbody>
      </table>

      <h3>Response</h3>
      <pre><code>{`{
  "results": [
    {
      "id": "msg-uuid-1",
      "content": "Fixed the authentication bug in the login handler",
      "conversation_id": "conv-uuid",
      "conversation_name": "backend-dev",
      "sender_name": "backend-dev",
      "created_at": "2026-04-15T10:30:00Z",
      "rank": 0.85
    },
    {
      "id": "msg-uuid-2",
      "content": "The auth module needs a fix for token expiration",
      "conversation_id": "conv-uuid-2",
      "conversation_name": "#dev-team",
      "sender_name": "Alice",
      "created_at": "2026-04-15T09:15:00Z",
      "rank": 0.62
    }
  ]
}`}</code></pre>

      <h2>Search Behavior</h2>
      <ul>
        <li><strong>Minimum query length</strong> &mdash; Queries must be at least 2 characters</li>
        <li><strong>Result limit</strong> &mdash; Results are limited to 50 matches, sorted by relevance</li>
        <li><strong>All conversations</strong> &mdash; Search spans all conversations the user has access to</li>
        <li><strong>Real-time updates</strong> &mdash; Newly sent messages are immediately searchable (no reindexing delay)</li>
      </ul>

      <div className="callout callout-info">
        <strong>Instant indexing</strong>
        Because the trigram index is maintained by PostgreSQL as a GIN index, new messages are searchable immediately after insertion. There is no background reindexing step or search lag.
      </div>

      <h2>Navigation</h2>
      <p>Clicking a search result:</p>
      <ol>
        <li>Switches to the conversation containing the matched message</li>
        <li>Scrolls the chat view to the exact message</li>
        <li>Highlights the matched message briefly for visibility</li>
      </ol>
      <p>This makes it easy to find and revisit specific messages across any conversation in the system.</p>

      <div className="callout callout-tip">
        <strong>Search tips</strong>
        Use specific keywords for better results. The trigram system handles partial matches well, so searching for &quot;auth bug&quot; will find messages containing &quot;authentication bug&quot; or &quot;auth bugfix&quot;.
      </div>

      <div className="docs-pager">
        <Link href="/docs/features/file-explorer">
          <span className="docs-pager-label">Previous</span>
          File Explorer &amp; Editor
        </Link>
        <Link href="/docs/features/cron-scheduler">
          <span className="docs-pager-label">Next</span>
          Cron Scheduler
        </Link>
      </div>
    </>
  );
}
