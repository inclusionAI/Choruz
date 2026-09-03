"use client";

import type { Principal, Conversation } from "../../lib/api/choruz-types";
import { Avatar } from "../ui/avatar";

export type ChatHeaderProps = {
  activeConv: Conversation | null;
  chatTitle: string;
  chatSubtitle: string;
  isAgentDm?: boolean;
  showDetail: boolean;
  wsStatus: string;
  onToggleSidebar: () => void;
  onToggleDetail: () => void;
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function ChatHeader({
  activeConv,
  chatTitle,
  chatSubtitle,
  isAgentDm = false,
  showDetail,
  wsStatus,
  onToggleSidebar,
  onToggleDetail,
}: ChatHeaderProps) {
  return (
    <>
      <div className={isAgentDm ? "chat-header is-agent" : "chat-header"}>
        <div className="chat-header-left">
          <button
            className="mobile-menu-btn"
            onClick={onToggleSidebar}
            title="Toggle sidebar"
            aria-label="Open menu"
          >
            {"\u2630"}
          </button>
          <div className="chat-header-info">
            {activeConv && <Avatar name={chatTitle} size="small" />}
            <div>
              <h1 className="chat-title">{chatTitle}</h1>
              <div className="chat-subtitle">{chatSubtitle}</div>
            </div>
          </div>
        </div>
        <div className="chat-header-actions">
          {activeConv && (
            <button
              className={showDetail ? "active" : ""}
              onClick={onToggleDetail}
              title="Toggle details"
            >
              i
            </button>
          )}
        </div>
      </div>

      {/* ---- Connection status banners ---- */}
      {wsStatus === "reconnecting" && (
        <div className="connection-banner connection-banner-reconnecting">
          <span>Reconnecting…</span>
          <button
            onClick={() => window.location.reload()}
            className="connection-banner-retry"
          >
            Retry
          </button>
        </div>
      )}
    </>
  );
}
