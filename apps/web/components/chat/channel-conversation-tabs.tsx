"use client";

import { useRef, type KeyboardEvent } from "react";

type ChannelConversationTabsProps = {
  pluginTabs: ReadonlyArray<{ id: "tasks"; label: string }>;
  activeView: "chat" | "tasks";
  onSelectView: (view: "chat" | "tasks") => void;
};

export function ChannelConversationTabs({
  pluginTabs,
  activeView,
  onSelectView,
}: ChannelConversationTabsProps) {
  const tabListRef = useRef<HTMLDivElement>(null);
  if (pluginTabs.length === 0) {
    return null;
  }

  const selectAdjacentTab = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const tabs = Array.from(
      tabListRef.current?.querySelectorAll<HTMLButtonElement>("[role=tab]") ?? [],
    );
    const current = tabs.indexOf(event.currentTarget);
    const nextIndex = event.key === "Home"
      ? 0
      : event.key === "End"
        ? tabs.length - 1
        : (current + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
    tabs[nextIndex]?.click();
    tabs[nextIndex]?.focus();
  };

  return (
    <div ref={tabListRef} className="channel-conversation-tabs" role="tablist" aria-label="Conversation views">
      <button
        id="conversation-chat-tab"
        type="button"
        role="tab"
        aria-selected={activeView === "chat"}
        aria-controls="conversation-chat-panel"
        tabIndex={activeView === "chat" ? 0 : -1}
        className={`channel-conversation-tab${activeView === "chat" ? " active" : ""}`}
        onClick={() => onSelectView("chat")}
        onKeyDown={selectAdjacentTab}
      >
        Chat
      </button>
      {pluginTabs.map((tab) => (
        <button
          key={tab.id}
          id={`conversation-${tab.id}-tab`}
          type="button"
          role="tab"
          aria-selected={activeView === tab.id}
          aria-controls={`conversation-${tab.id}-panel`}
          tabIndex={activeView === tab.id ? 0 : -1}
          className={`channel-conversation-tab${activeView === tab.id ? " active" : ""}`}
          onClick={() => onSelectView(tab.id)}
          onKeyDown={selectAdjacentTab}
        >
          {tab.label}
        </button>
      ))}
    </div>
  );
}
