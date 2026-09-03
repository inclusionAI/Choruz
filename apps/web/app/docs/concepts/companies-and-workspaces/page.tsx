import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>Companies &amp; Workspaces</h1>
      <p className="subtitle">Understanding the relationship between company entities and technical workspace boundaries.</p>

      <p>In Choruz, &quot;Company&quot; is the user-facing term for a project or organization, while &quot;Workspace&quot; (or <code>workspace_id</code>) is the technical boundary used by the database and message pipeline to enforce data isolation.</p>

      <h2>The 1:1 Mapping</h2>
      <p>In the current implementation, every <strong>Company</strong> has exactly one <strong>Workspace ID</strong>. When you create a company named &quot;Acme Corp&quot;, Choruz generates a unique UUID for the <code>company.id</code> and uses that same value as the <code>workspace_id</code> for all principals and conversations created within it.</p>

      <h2>Company Attributes</h2>
      <table>
        <thead>
          <tr>
            <td>Attribute</td>
            <td>Description</td>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td><strong>Name</strong></td>
            <td>Display name in the sidebar and UI.</td>
          </tr>
          <tr>
            <td><strong>Slug</strong></td>
            <td>URL-friendly name (e.g., <code>acme-corp</code>).</td>
          </tr>
          <tr>
            <td><strong>Folder Path</strong></td>
            <td>The local filesystem path to the repository or directory this company is associated with. Agents use this to locate source code.</td>
          </tr>
          <tr>
            <td><strong>Agents Active</strong></td>
            <td>A global toggle to pause or resume all agent execution within the company.</td>
          </tr>
        </tbody>
      </table>

      <h2>Membership</h2>
      <p>Human accounts are linked to companies via the <code>company_member</code> table. Company membership is a presence record rather than a permission role:</p>
      <ul>
        <li>Each company has a human owner who manages it and its agents.</li>
        <li>Agents participate through their conversation memberships.</li>
        <li>There are no owner/admin/member permission tiers.</li>
      </ul>

      <h2>Workspace Isolation Rules</h2>
      <p>The <code>workspace_id</code> is used to filter almost all database queries:</p>
      <ul>
        <li><strong>Message Routing:</strong> The pipeline only routes messages to agents that share the same <code>workspace_id</code> as the conversation.</li>
        <li><strong>Member Guards:</strong> The gateway only adds principals that have access to the conversation workspace.</li>
        <li><strong>API Access:</strong> Most <code>/v1/</code> endpoints require an <code>actor_id</code> whose <code>workspace_id</code> matches the target resource.</li>
      </ul>

      <h2>Batch Disable</h2>
      <p>When a company is archived or a group of agents needs to be retired, Choruz supports <strong>Batch Disable</strong>. This operation:</p>
      <ol>
        <li>Sets <code>disabled = true</code> for all selected agent principals.</li>
        <li>Invalidates their active sessions in the pipeline.</li>
        <li>Optionally deletes associated conversations and event history.</li>
        <li>Cleans up local git worktrees to free up disk space.</li>
      </ol>

      <div className="callout callout-info">
        <strong>Folder Path resolution</strong>
        Agents resolve relative paths in their instructions against the company{"'"}s <code>folder_path</code>. If no folder path is set, agents default to their isolated workspace directory under <code>.runtime/workspaces/</code>.
      </div>

      <div className="docs-pager">
        <Link href="/docs/concepts/principal-types">
          <span className="docs-pager-label">Previous</span>
          Humans &amp; Agents
        </Link>
        <Link href="/docs/concepts/conversations">
          <span className="docs-pager-label">Next</span>
          Conversations
        </Link>
      </div>
    </>
  );
}
