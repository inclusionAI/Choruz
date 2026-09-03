export const CORE_PROTOCOL_PLACEHOLDER = "{{CHORUZ_CORE_PROTOCOL}}";
export const STANDARD_EXTENSIONS_PLACEHOLDER = "{{CHORUZ_STANDARD_EXTENSIONS}}";

export const CORE_PROTOCOL_FILE = "core-protocol.md";

export const STANDARD_EXTENSION_FILES = [
  "extensions/multi-agent-collaboration.md",
  "extensions/command-results.md",
  "extensions/file-sharing.md",
  "extensions/agent-management.md",
  "extensions/group-management.md",
  "extensions/scheduled-tasks.md",
  "extensions/collaboration-practices.md",
] as const;

export type AgentInstructionFragments = {
  coreProtocol: string;
  standardExtensions: readonly string[];
};

export function composeAgentInstructionTemplate(
  template: string,
  role: string,
  fragments: AgentInstructionFragments,
): string {
  const replacements = new Map([
    [CORE_PROTOCOL_PLACEHOLDER, fragments.coreProtocol.trim()],
    [
      STANDARD_EXTENSIONS_PLACEHOLDER,
      fragments.standardExtensions.map((value) => value.trim()).join("\n\n---\n\n"),
    ],
    ["{{AGENT_INSTRUCTIONS}}", role.trim()],
  ]);

  let rendered = template;
  for (const [placeholder, value] of replacements) {
    if (!rendered.includes(placeholder)) {
      throw new Error(`Agent instructions template is missing ${placeholder}`);
    }
    rendered = rendered.replace(placeholder, value);
  }
  return `${rendered.trim()}\n`;
}
