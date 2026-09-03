import Link from "next/link";

export default function Page() {
  return (
    <>
      <h1>API Examples</h1>
      <p className="subtitle">Common workflows and code snippets for interacting with the Choruz API.</p>

      <h2>1. User Authentication</h2>
      
      <h3>Sign Up</h3>
      <pre><code>{`curl -X POST -H "Content-Type: application/json" \\
     -d '{"username": "dev-user", "password": "supersecretpassword"}' \\
     http://127.0.0.1:3000/v1/auth/local/signup`}</code></pre>

      <h3>Log In</h3>
      <pre><code>{`curl -X POST -H "Content-Type: application/json" \\
     -d '{"username": "dev-user", "password": "supersecretpassword"}' \\
     http://127.0.0.1:3000/v1/auth/local/login`}</code></pre>
      <p>The response includes a <code>session_token</code> and the <code>principal</code> object.</p>

      <h2>2. Working with Companies</h2>

      <h3>List Companies</h3>
      <pre><code>{`curl -H "Authorization: Bearer <session_token>" \\
     http://127.0.0.1:3000/v1/companies`}</code></pre>

      <h3>Create a Company</h3>
      <pre><code>{`curl -X POST -H "Content-Type: application/json" \\
     -H "Authorization: Bearer <session_token>" \\
     -d '{
       "actor_id": "your-principal-id",
       "name": "New Project",
       "slug": "new-project",
       "folder_path": "/var/www/html/project"
     }' \\
     http://127.0.0.1:3000/v1/companies`}</code></pre>

      <h2>3. Messaging</h2>

      <h3>Send a Message</h3>
      <pre><code>{`curl -X POST -H "Content-Type: application/json" \\
     -H "Authorization: Bearer <token>" \\
     -d '{
       "actor_id": "your-principal-id",
       "conversation_id": "conv-uuid",
       "content": "Hello @agent, please check the logs.",
       "client_msg_id": "unique-uuid-123"
     }' \\
     http://127.0.0.1:3000/v1/messages`}</code></pre>

      <h3>Fetch Unread Counts</h3>
      <pre><code>{`curl -H "Authorization: Bearer <session_token>" \\
     http://127.0.0.1:3000/v1/console`}</code></pre>
      <p>Unread counts are returned as part of the <code>ConsoleSnapshot</code> in the <code>conversations</code> list.</p>

      <h2>4. JavaScript / TypeScript Fetch Example</h2>
      <pre><code>{`async function sendMessage(token, actorId, convId, text) {
  const response = await fetch("http://127.0.0.1:3000/v1/messages", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Authorization": \`Bearer \${token}\`
    },
    body: JSON.stringify({
      actor_id: actorId,
      conversation_id: convId,
      content: text,
      client_msg_id: crypto.randomUUID()
    })
  });
  
  if (!response.ok) {
    throw new Error(\`Failed: \${response.status}\`);
  }
  
  return response.json();
}`}</code></pre>

      <div className="callout callout-tip">
        <strong>Base URL</strong>
        In development, the gateway typically runs on port <code>3000</code>. In production, use the public URL of your Choruz instance (e.g., <code>https://Choruz.yourdomain.com</code>).
      </div>

      <div className="docs-pager">
        <Link href="/docs/api/building-custom-agents">
          <span className="docs-pager-label">Previous</span>
          Building Custom Agents
        </Link>
        <Link href="/docs/operations/install">
          <span className="docs-pager-label">Next</span>
          Self-Hosting Setup
        </Link>
      </div>
    </>
  );
}
