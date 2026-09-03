import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>File Attachments</h1>
      <p className="subtitle">Upload, download, and display file attachments in conversations with inline image rendering and authenticated proxy access.</p>

      <h2>Overview</h2>
      <p>Choruz supports file attachments in chat messages. You can upload files via the REST API, reference them in markdown for inline rendering, and download them through an authenticated proxy. Images are rendered directly in the chat view.</p>

      <div className="docs-screenshot">
        <img src="/docs-img/group-chat-image.png" alt="Rendered image in group chat" />
        <div className="docs-screenshot-caption">A group chat with an inline-rendered PNG image attachment</div>
      </div>

      <h2>Uploading Attachments</h2>
      <p>Upload files by sending a base64-encoded JSON payload to the attachments endpoint:</p>

      <pre><code>{`POST /v1/attachments
Content-Type: application/json

{
  "filename": "screenshot.png",
  "content_type": "image/png",
  "data": "iVBORw0KGgoAAAANSUhEUgAA..."
}`}</code></pre>

      <h3>Response</h3>
      <pre><code>{`{
  "id": "att-uuid-1",
  "filename": "screenshot.png",
  "content_type": "image/png",
  "size": 24576,
  "created_at": "2026-04-15T10:30:00Z"
}`}</code></pre>

      <p>The returned <code>id</code> can be used to reference the attachment in messages or download it later.</p>

      <h2>Downloading Attachments</h2>
      <p>Retrieve an uploaded file by its ID:</p>

      <pre><code>{`GET /v1/attachments/{id}`}</code></pre>

      <p>The response streams the file with the correct <code>Content-Type</code> header. For images, browsers can render them directly.</p>

      <h2>Inline Image Rendering</h2>
      <p>To display an image inline in a chat message, use standard markdown image syntax with the attachment URL:</p>

      <pre><code>{`![screenshot](/v1/attachments/att-uuid-1)`}</code></pre>

      <p>The chat renderer detects image URLs pointing to <code>/v1/attachments/</code> and renders them as inline images with automatic sizing and click-to-expand.</p>

      <div className="callout callout-info">
        <strong>Automatic rendering</strong>
        When a message contains a markdown image reference to a Choruz attachment, the frontend renders it as a full inline image rather than a link. This works for PNG, JPEG, GIF, and WebP formats.
      </div>

      <h2>Next.js Proxy</h2>
      <p>The frontend proxies attachment requests through a Next.js API route for authentication:</p>

      <pre><code>{`GET /api/attachments/[id]`}</code></pre>

      <p>This proxy route:</p>
      <ul>
        <li>Validates the user{"'"}s session before serving the file</li>
        <li>Forwards the request to the gateway{"'"}s <code>/v1/attachments/{"{id}"}</code> endpoint</li>
        <li>Preserves the original <code>Content-Type</code> and <code>Content-Disposition</code> headers</li>
        <li>Ensures attachments are not accessible without authentication</li>
      </ul>

      <h2>Supported Formats</h2>
      <table>
        <thead><tr><th>Category</th><th>Formats</th><th>Inline Preview</th></tr></thead>
        <tbody>
          <tr><td>Images</td><td>PNG, JPEG, GIF, WebP, SVG</td><td>Yes &mdash; rendered inline in chat</td></tr>
          <tr><td>Documents</td><td>PDF, TXT, MD</td><td>No &mdash; download link shown</td></tr>
          <tr><td>Code</td><td>Any text file</td><td>No &mdash; download link shown</td></tr>
          <tr><td>Archives</td><td>ZIP, TAR, GZ</td><td>No &mdash; download link shown</td></tr>
        </tbody>
      </table>

      <h2>Storage</h2>
      <p>Attachments are stored on the local filesystem in the directory specified by the <code>CHORUZ_ATTACHMENT_DIR</code> environment variable. Each file is stored with its UUID as the filename to prevent collisions.</p>

      <div className="callout callout-tip">
        <strong>Attachment directory</strong>
        Make sure the <code>CHORUZ_ATTACHMENT_DIR</code> directory exists and is writable by the Choruz process. If not set, attachments default to <code>./attachments</code> relative to the gateway{"'"}s working directory.
      </div>

      <h2>API Reference</h2>
      <table>
        <thead><tr><th>Operation</th><th>Method</th><th>Endpoint</th></tr></thead>
        <tbody>
          <tr><td>Upload attachment</td><td><code>POST</code></td><td><code>/v1/attachments</code></td></tr>
          <tr><td>Download attachment</td><td><code>GET</code></td><td><code>/v1/attachments/:id</code></td></tr>
          <tr><td>Proxy (frontend)</td><td><code>GET</code></td><td><code>/api/attachments/:id</code></td></tr>
        </tbody>
      </table>

      <div className="docs-pager">
        <Link href="/docs/features/chat">
          <span className="docs-pager-label">Previous</span>
          Chat
        </Link>
        <Link href="/docs/features/file-explorer">
          <span className="docs-pager-label">Next</span>
          File Explorer &amp; Editor
        </Link>
      </div>
    </>
  );
}
