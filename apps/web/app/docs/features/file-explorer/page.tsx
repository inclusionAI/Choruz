import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Files</h1>
      <p className="subtitle">Browse and edit the folder attached to the active company.</p>

      <h2>Show the file tree</h2>
      <p>Set <strong>Workspace Folder</strong> when you create a company. Its file tree then appears at the top of the left sidebar. If no tree is visible, create or switch to a company that has a workspace folder.</p>

      <h2>Browse and refresh</h2>
      <p>Expand folders in the sidebar and select a file to open it in the editor. Use the refresh button after an agent or an external tool creates, renames, or removes files.</p>
      <ul>
        <li>Directories load when you expand them.</li>
        <li>Folders are listed before files, then sorted alphabetically.</li>
        <li>The tree reads the filesystem of the computer running the agent.</li>
      </ul>

      <h2>Edit a file</h2>
      <p>Selected files open in the CodeMirror editor. You can keep several files open in tabs, save changes back to the workspace, and preview Markdown files.</p>
      <ul>
        <li>The Choruz server process must have permission to read or write the path.</li>
        <li>Saving in the browser changes the real file; it does not create a separate copy.</li>
        <li>Use Git or another version-control tool when you need reviewable history.</li>
      </ul>

      <div className="callout callout-warn">
        <strong>Remote control</strong>
        A paired browser can operate this editor, but raw file contents are not relayed as part of remote chat. Keep the host computer and paired devices under your control.
      </div>

      <div className="docs-pager">
        <Link href="/docs/features/attachments">
          <span className="docs-pager-label">Previous</span>
          File Attachments
        </Link>
        <Link href="/docs/features/search">
          <span className="docs-pager-label">Next</span>
          Search
        </Link>
      </div>
    </>
  );
}
