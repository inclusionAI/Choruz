"use client";

import { usePathname } from "next/navigation";
import Link from "next/link";
import "./docs.css";

const NAV = [
  {
    title: "Start",
    items: [
      { href: "/docs", label: "Overview" },
      { href: "/docs/getting-started/installation", label: "Install & Run" },
      { href: "/docs/getting-started/quickstart", label: "First 10 Minutes" },
      { href: "/docs/getting-started/first-company", label: "Create a Company" },
      { href: "/docs/getting-started/first-agent", label: "Create an Agent" },
    ],
  },
  {
    title: "Use Choruz",
    items: [
      { href: "/docs/features/chat", label: "Chat" },
      { href: "/docs/concepts/conversations", label: "Groups & Conversations" },
      { href: "/docs/concepts/mentions", label: "@mention Agents" },
      { href: "/docs/features/file-explorer", label: "Files" },
      { href: "/docs/features/search", label: "Search" },
      { href: "/docs/features/remote-control", label: "Remote Control" },
      { href: "/docs/features/server-management", label: "Remote Servers (SSH)" },
    ],
  },
  {
    title: "Agents",
    items: [
      { href: "/docs/agents/drivers", label: "Drivers & Accounts" },
      { href: "/docs/agents/instructions", label: "Instructions" },
      { href: "/docs/agents/skills", label: "Skills" },
      { href: "/docs/agents/sub-agents", label: "Sub-agents" },
      { href: "/docs/agents/ai-manager", label: "AI Manager" },
      { href: "/docs/features/cron-scheduler", label: "Scheduled Work" },
    ],
  },
  {
    title: "Integrate",
    items: [
      { href: "/docs/api/authentication", label: "Authentication" },
      { href: "/docs/api/rest", label: "REST API" },
      { href: "/docs/api/websocket", label: "WebSocket" },
      { href: "/docs/api/webhooks", label: "Webhook Agents" },
      { href: "/docs/api/building-custom-agents", label: "Custom Agents" },
    ],
  },
  {
    title: "Operate",
    items: [
      { href: "/docs/operations/deployment", label: "Deployment" },
      { href: "/docs/operations/env-vars", label: "Environment Variables" },
      { href: "/docs/operations/backup", label: "Backup & Restore" },
      { href: "/docs/troubleshooting/agent-not-responding", label: "Agent Not Responding" },
      { href: "/docs/troubleshooting/common-errors", label: "Common Errors" },
    ],
  },
];

export default function DocsLayout({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();

  return (
    <div className="docs-root">
      {/* ── Top Nav ── */}
      <nav className="docs-nav">
        <Link href="/docs" className="docs-logo">
          Choruz <span className="docs-logo-tag">docs</span>
        </Link>
        <div className="docs-nav-links">
          <a href="https://github.com/jcguo123/Choruz" target="_blank" rel="noreferrer">GitHub</a>
          <Link href="/dashboard">Dashboard</Link>
        </div>
      </nav>

      <div className="docs-body">
        {/* ── Sidebar ── */}
        <aside className="docs-sidebar">
          {NAV.map((section) => (
            <div key={section.title} className="docs-sidebar-section">
              <div className="docs-sidebar-title">{section.title}</div>
              {section.items.map((item) => (
                <Link
                  key={item.href}
                  href={item.href}
                  className={`docs-sidebar-link${pathname === item.href ? " active" : ""}`}
                >
                  {item.label}
                </Link>
              ))}
            </div>
          ))}
        </aside>

        {/* ── Content ── */}
        <main className="docs-content">
          {children}
        </main>
      </div>
    </div>
  );
}
