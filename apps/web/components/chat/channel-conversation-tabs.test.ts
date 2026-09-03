import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ChannelConversationTabs } from "./channel-conversation-tabs";

describe("ChannelConversationTabs", () => {
  it("renders no Tasks tab while channel tasks are disabled", () => {
    const html = renderToStaticMarkup(
      createElement(ChannelConversationTabs, {
        pluginTabs: [],
        activeView: "chat",
        onSelectView: () => {},
      }),
    );

    expect(html).toBe("");
    expect(html).not.toContain("Tasks");
  });

  it("renders Chat and Tasks tabs when channel tasks are enabled", () => {
    const html = renderToStaticMarkup(
      createElement(ChannelConversationTabs, {
        pluginTabs: [{ id: "tasks", label: "Tasks" }],
        activeView: "chat",
        onSelectView: () => {},
      }),
    );

    expect(html).toContain("Chat");
    expect(html).toContain("Tasks");
    expect(html).toContain('aria-label="Conversation views"');
    expect(html).toContain('role="tablist"');
    expect(html).toContain('role="tab"');
    expect(html).toContain('aria-selected="true"');
    expect(html).toContain('aria-controls="conversation-chat-panel"');
    expect(html).toContain('id="conversation-chat-tab"');
    expect(html).toContain('id="conversation-tasks-tab"');
    expect(html).toContain('tabindex="0"');
    expect(html).toContain('tabindex="-1"');
  });
});
