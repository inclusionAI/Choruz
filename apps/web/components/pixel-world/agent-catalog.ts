/**
 * apps/web/components/pixel-world/agent-catalog.ts
 *
 * Agent visual descriptor catalog.
 * Maps 17 legacy atlas silhouettes + 20 Choruz roster sheets into 100+
 * addressable agents by combining base assets with palette-swap target hexes.
 */

// Legacy atlas frame names. The source sheets are packed into agent_atlas.png;
// runtime rendering addresses the packed regions directly.
export const MASTER_ASSETS = {
  CEO: 'ceo',
  CHEF: 'chef',
  CODER: 'coder',
  MANAGER: 'corp_manager',
  ANALYST: 'data_analyst',
  JANITOR: 'janitor',
  MINER: 'miner',
  PILOT: 'pilot',
  SERVER_ENG: 'server_engineer',
  SYSADMIN: 'sysadmin',
  BOTANIST: 'botanist',
  DESIGNER: 'creative_designer',
  EXPLORER: 'acp_explorer',
  HR: 'hr',
  LIBRARIAN: 'librarian',
  MEDIC: 'medic',
  SECURITY: 'security',
};

/**
 * Choruz roster — 20 redesigned agent sprite sheets.
 *
 * Generated via Nano Banana Pro (initial idle pose + status animation strips)
 * + PixelLab `/rotate` and `/animate-with-text` (walk cycles for four cardinal
 * directions). Each sheet is a 192×384 PNG laid out as 8 rows × 4 cols of
 * 48px cells:
 *   row 0 walk-down / south   row 1 walk-right / east
 *   row 2 walk-up   / north   row 3 walk-left  / west
 *   row 4 typing   (front-only)   row 5 thinking  (front-only)
 *   row 6 sitting  (front-only)   row 7 talking   (front-only)
 *
 * Keyed by the agent roster id.
 */
export const CHORUZ_AGENT_SHEETS: Record<string, string> = {
  // Humans (10)
  founder:           '/sprites/generated/agents/sheets/founder.png',
  product_lead:      '/sprites/generated/agents/sheets/product_lead.png',
  engineer:          '/sprites/generated/agents/sheets/engineer.png',
  designer:          '/sprites/generated/agents/sheets/designer.png',
  data_analyst:      '/sprites/generated/agents/sheets/data_analyst.png',
  people_ops:        '/sprites/generated/agents/sheets/people_ops.png',
  community_manager: '/sprites/generated/agents/sheets/community_manager.png',
  writer:            '/sprites/generated/agents/sheets/writer.png',
  researcher:        '/sprites/generated/agents/sheets/researcher.png',
  facilities_lead:   '/sprites/generated/agents/sheets/facilities_lead.png',
  // AI teammates (10)
  code_assistant:    '/sprites/generated/agents/sheets/code_assistant.png',
  research_bot:      '/sprites/generated/agents/sheets/research_bot.png',
  data_wrangler:     '/sprites/generated/agents/sheets/data_wrangler.png',
  docs_keeper:       '/sprites/generated/agents/sheets/docs_keeper.png',
  qa_bot:            '/sprites/generated/agents/sheets/qa_bot.png',
  devops_agent:      '/sprites/generated/agents/sheets/devops_agent.png',
  scheduler:         '/sprites/generated/agents/sheets/scheduler.png',
  orchestrator:      '/sprites/generated/agents/sheets/orchestrator.png',
  archivist:         '/sprites/generated/agents/sheets/archivist.png',
  triage_bot:        '/sprites/generated/agents/sheets/triage_bot.png',
};

/** True when the given asset path points to a Choruz roster sheet rather than the legacy atlas. */
export function isChoruzRosterAsset(asset: string | undefined): boolean {
  return !!asset && asset.startsWith('/sprites/generated/agents/sheets/');
}

export const COLOR_PALETTE = [
  '#E63946', '#F4A261', '#E9C46A', '#2A9D8F', '#264653',
  '#1D3557', '#457B9D', '#A8DADC', '#81B29A', '#3D405B',
  '#F2CC8F', '#E07A5F', '#9B5DE5', '#F15BB5', '#00BBF9'
];

export interface AgentDescriptor {
  id: string;
  name: string;
  masterAsset: string;
  primaryColorHex: string; // The color the engine will tint their clothes to
}

// Pseudo-random generation based on index
function generateGenericAgent(index: number): AgentDescriptor {
  const masterKeys = Object.keys(MASTER_ASSETS) as Array<keyof typeof MASTER_ASSETS>;
  
  // Deterministic but "random" looking distribution
  const masterChoice = masterKeys[(index * 13) % masterKeys.length];
  const colorChoice = COLOR_PALETTE[(index * 17) % COLOR_PALETTE.length];
  
  return {
    id: `AGENT_${index.toString().padStart(3, '0')}`,
    name: `Unit-${index * 83}`,
    masterAsset: MASTER_ASSETS[masterChoice],
    primaryColorHex: colorChoice,
  };
}

// Build the massive 100 roster lookup DB securely in memory
const internalCatalog: Map<string, AgentDescriptor> = new Map();

// 1. First, insert the System Specific Hero Agents
internalCatalog.set('claude_terminal', { id: 'claude_terminal', name: 'Terminal Host', masterAsset: MASTER_ASSETS.SYSADMIN, primaryColorHex: '#00FF00' });
internalCatalog.set('codex_app_server', { id: 'codex_app_server', name: 'App Server', masterAsset: MASTER_ASSETS.SERVER_ENG, primaryColorHex: '#FF7700' });
internalCatalog.set('acp', { id: 'acp', name: 'ACP Protocol', masterAsset: MASTER_ASSETS.PILOT, primaryColorHex: '#55CCFF' });
internalCatalog.set('claude_print', { id: 'claude_print', name: 'Librarian AI', masterAsset: MASTER_ASSETS.CEO, primaryColorHex: '#AA44FF' });
internalCatalog.set('codex_exec', { id: 'codex_exec', name: 'Execution Env', masterAsset: MASTER_ASSETS.CODER, primaryColorHex: '#4444FF' });
internalCatalog.set('pi_terminal', { id: 'pi_terminal', name: 'Pi Agent', masterAsset: MASTER_ASSETS.PILOT, primaryColorHex: '#F5A623' });
internalCatalog.set('grok_terminal', { id: 'grok_terminal', name: 'Grok Build', masterAsset: MASTER_ASSETS.SERVER_ENG, primaryColorHex: '#E6E6E6' });
internalCatalog.set('opencode_terminal', { id: 'opencode_terminal', name: 'OpenCode', masterAsset: MASTER_ASSETS.CODER, primaryColorHex: '#66D9EF' });

// 2. Insert the 100 Generic "Human" Paper Doll stand-ins
for (let i = 1; i <= 100; i++) {
  const genericAgent = generateGenericAgent(i);
  internalCatalog.set(genericAgent.id, genericAgent);
}

// 3. Register the 20 Choruz roster agents. These use their own 192×384 sheets
//    (not the legacy atlas), so primaryColorHex is a neutral #ffffff meaning
//    "no palette swap" — the sheet already carries the final art colors.
for (const [id, path] of Object.entries(CHORUZ_AGENT_SHEETS)) {
  const readable = id.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());
  internalCatalog.set(id, {
    id,
    name: readable,
    masterAsset: path,
    primaryColorHex: '#ffffff',
  });
}

// 4. The local "player" avatar — i.e. the logged-in user moving around the
//    pixel world — also needs to pick one of the 192×384 roster sheets
//    rather than falling through to the hash-based legacy-atlas bucket.
//    Otherwise the user shows up as whatever generic AGENT_NNN their id
//    hashes into (we were seeing "soldier"), and the 8-row status
//    animations (typing / thinking / sitting / talking) silently don't
//    apply because the legacy atlas only has the 4 walk rows.
//
//    `pixel-world-store.ts` passes the literal string `'player'` as the
//    agent id to `loadAgentTexture()`, so we register that exact key.
internalCatalog.set('player', {
  id: 'player',
  name: 'You',
  masterAsset: CHORUZ_AGENT_SHEETS.founder,
  primaryColorHex: '#ffffff',
});

/**
 * Accessor function used by the rendering engine
 */
export function getAgentVisualDescriptor(agentId: string): AgentDescriptor {
  // If we have an exact match in the dictionary
  if (internalCatalog.has(agentId)) {
    return internalCatalog.get(agentId)!;
  }
  
  // Deterministic fallback for totally unknown IDs hashing to a 1-100 generic avatar
  let h = 0;
  for (let i = 0; i < agentId.length; i++) h = (Math.imul(31, h) + agentId.charCodeAt(i)) | 0;
  const hashObj = (Math.abs(h) % 100) + 1; // 1-100, matching AGENT_001..AGENT_100
  return internalCatalog.get(`AGENT_${hashObj.toString().padStart(3, '0')}`)!;
}
