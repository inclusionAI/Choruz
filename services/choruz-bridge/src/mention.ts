/**
 * @mention format conversion between platforms and Choruz.
 *
 * Slack uses <@U1234>, Telegram uses @username, Choruz uses @agent-name.
 */

/**
 * Convert Slack-style mentions (<@U1234>) to Choruz-style (@agent-name).
 *
 * `agentNameMap` maps Slack user IDs to Choruz agent names.
 * Unknown <@Uxxx> mentions are left as-is.
 */
export function slackMentionToChoruz(
  text: string,
  _botUserId: string,
  agentNameMap: Map<string, string>,
): string {
  return text.replace(/<@(U[A-Z0-9]+)>/g, (_match, userId: string) => {
    const agentName = agentNameMap.get(userId);
    if (agentName) return `@${agentName}`;
    return `<@${userId}>`;
  });
}

/**
 * Convert Telegram @username mentions to Choruz @agent-name.
 *
 * V1 simple approach: strip the bot's own @username mention (it's
 * redundant since the message already reached the bot). Other @xxx
 * mentions are kept as-is since Choruz matches by @agent-name.
 */
export function telegramMentionToChoruz(
  text: string,
  botUsername: string,
): string {
  if (!botUsername) return text;

  // Remove the bot's own @mention (case-insensitive)
  const pattern = new RegExp(`@${escapeRegExp(botUsername)}\\b`, 'gi');
  return text.replace(pattern, '').trim();
}

/**
 * Convert Choruz @agent-name to platform-ready text.
 *
 * V1: pass through unchanged. Platforms will display @agent-name as
 * plain text, which is acceptable. Rich reverse-mapping can be added later.
 */
export function choruzMentionToPlatform(text: string): string {
  return text;
}

function escapeRegExp(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
