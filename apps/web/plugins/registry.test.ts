import { describe, expect, it } from "vitest";

import type { HostPluginManifest } from "../lib/api/choruz-types";
import { resolveClientPluginIds } from "./registry";

const kanbanHost: HostPluginManifest = {
  id: "kanban",
  version: "1",
  host_capabilities: ["channel-task-api", "channel-task-events"],
  client_capabilities: ["conversation-tab", "message-action"],
};

const additionalHosts: HostPluginManifest[] = [
  {
    id: "workspace-git",
    version: "1",
    host_capabilities: ["workspace-repository-access"],
    client_capabilities: ["detail-tab", "git-graph"],
  },
  {
    id: "remote-ssh",
    version: "1",
    host_capabilities: ["ssh-host-discovery", "ssh-tunnel-api", "remote-choruz-connect"],
    client_capabilities: ["sidebar-action", "modal"],
  },
  {
    id: "remote-control",
    version: "1",
    host_capabilities: ["opaque-pairing-credential", "remote-device-api", "cloud-gateway", "end-to-end-encryption"],
    client_capabilities: ["sidebar-action", "modal", "web-dashboard"],
  },
  {
    id: "agent-skills",
    version: "1",
    host_capabilities: ["agent-workspace-access"],
    client_capabilities: ["agent-detail-tab", "agent-provisioning"],
  },
  {
    id: "mathcode",
    version: "1",
    host_capabilities: ["mathcode-agent-driver"],
    client_capabilities: ["agent-provisioning"],
  },
];

describe("resolveClientPluginIds", () => {
  it("activates a client plugin only when its matching host contract is present", () => {
    expect([...resolveClientPluginIds([kanbanHost])]).toEqual(["kanban"]);
    expect([...resolveClientPluginIds([])]).toEqual([]);
  });

  it("rejects incompatible versions and incomplete capabilities", () => {
    expect([...resolveClientPluginIds([{ ...kanbanHost, version: "2" }])]).toEqual([]);
    expect([
      ...resolveClientPluginIds([{ ...kanbanHost, host_capabilities: ["channel-task-api"] }]),
    ]).toEqual([]);
  });

  it("installs every compatible feature plugin on the same core registry", () => {
    expect([...resolveClientPluginIds([kanbanHost, ...additionalHosts])]).toEqual([
      "kanban",
      "workspace-git",
      "remote-ssh",
      "remote-control",
      "agent-skills",
      "mathcode",
    ]);
  });

  it("ignores host plugins that this client does not implement", () => {
    expect([
      ...resolveClientPluginIds([
        {
          id: "future-plugin",
          version: "1",
          host_capabilities: [],
          client_capabilities: [],
        },
      ]),
    ]).toEqual([]);
  });
});
