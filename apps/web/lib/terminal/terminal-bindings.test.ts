import { describe, expect, it } from "vitest";

import type { Conversation, Principal, RuntimeBindingInfo } from "../api/choruz-types";
import { agentPeerId, bindingMachineLabel, bindingUsesTerminalTranscript, findTerminalBinding, openTerminalBindings } from "./terminal-bindings";

const agent = (id: string): Principal => ({ id, name: id, principal_type: "agent" }) as unknown as Principal;
const direct = (id: string, peer: string): Conversation =>
  ({ id, conversation_type: "direct", members: { me: {}, [peer]: {} } }) as unknown as Conversation;
const binding = (partial: Partial<RuntimeBindingInfo>): RuntimeBindingInfo =>
  ({ id: "b", driver_type: "claude_terminal", interaction_mode: "terminal", conversation_type: "direct", ...partial }) as unknown as RuntimeBindingInfo;

const agents = [agent("ada")];

describe("bindingUsesTerminalTranscript", () => {
  it("keeps imported native sessions in the dedicated terminal UI", () => {
    expect(bindingUsesTerminalTranscript({ interaction_mode: "terminal" })).toBe(true);
    expect(bindingUsesTerminalTranscript({ interaction_mode: "terminal" })).toBe(true);
  });

  it("trusts the gateway's interaction_mode, so a plugin driver needs no client list", () => {
    expect(bindingUsesTerminalTranscript({ interaction_mode: "terminal" })).toBe(true);
  });

  it("excludes message-mode bindings and bindings without a mode", () => {
    expect(bindingUsesTerminalTranscript({ interaction_mode: "message" })).toBe(false);
    expect(bindingUsesTerminalTranscript({})).toBe(false);
    expect(bindingUsesTerminalTranscript({ interaction_mode: null })).toBe(false);
  });
});

describe("bindingMachineLabel", () => {
  const hosts = [{ id: "host-1", name: "Build Server West" }];

  it("names the paired runtime host", () => {
    expect(bindingMachineLabel({ runtime_host_id: "host-1" }, hosts)).toBe("Build Server West");
  });

  it("falls back for a host this browser has not loaded", () => {
    expect(bindingMachineLabel({ runtime_host_id: "host-9" }, hosts)).toBe("Remote machine");
  });

  it("calls a binding without a host this computer", () => {
    expect(bindingMachineLabel({ runtime_host_id: null }, hosts)).toBe("This computer");
    expect(bindingMachineLabel({}, hosts)).toBe("This computer");
    expect(bindingMachineLabel(undefined, hosts)).toBe("This computer");
  });
});

describe("agentPeerId", () => {
  it("returns the agent on the other side of a direct conversation", () => {
    expect(agentPeerId(direct("c1", "ada"), "me", agents)).toBe("ada");
  });

  it("is null for humans and groups", () => {
    expect(agentPeerId(direct("c1", "pat"), "me", agents)).toBeNull();
    const group = { id: "g", conversation_type: "group", members: { me: {}, ada: {} } } as unknown as Conversation;
    expect(agentPeerId(group, "me", agents)).toBeNull();
  });
});

describe("findTerminalBinding", () => {
  const exact = binding({ id: "exact", conversation_id: "c1", agent_principal_id: "ada", conversation_type: "group" });
  const agentDirect = binding({ id: "direct", conversation_id: "other", agent_principal_id: "ada" });
  const any = binding({ id: "any", conversation_id: "x", agent_principal_id: "ada", conversation_type: "group" });

  it("prefers the binding on this conversation, then the agent's direct binding, then any", () => {
    expect(findTerminalBinding([any, agentDirect, exact], "c1", "ada")?.id).toBe("exact");
    expect(findTerminalBinding([any, agentDirect], "c1", "ada")?.id).toBe("direct");
    expect(findTerminalBinding([any], "c1", "ada")?.id).toBe("any");
  });
});

describe("openTerminalBindings", () => {
  it("maps each open agent DM to its binding once, in tab order", () => {
    const conversations = [direct("c1", "ada"), direct("c2", "ada"), direct("c3", "pat")];
    const bindings = [
      binding({ id: "b1", conversation_id: "c1", agent_principal_id: "ada" }),
      binding({ id: "b2", conversation_id: "c2", agent_principal_id: "ada" }),
    ];
    expect(openTerminalBindings(["c2", "c1", "c2", "c3", "missing"], conversations, agents, "me", bindings)).toEqual([
      { convId: "c2", bindingId: "b2" },
      { convId: "c1", bindingId: "b1" },
    ]);
  });
});
