import { Bot } from 'grammy';
import type { ChoruzClient } from '../choruz-client.js';
import type { MappingStore } from '../mapping-store.js';
import { telegramMentionToChoruz } from '../mention.js';

export class TelegramAdapter {
  private bot: Bot;
  private choruz: ChoruzClient;
  private mappings: MappingStore;
  private bridgePrincipalId: string;

  constructor(params: {
    botToken: string;
    choruz: ChoruzClient;
    mappings: MappingStore;
    bridgePrincipalId: string;
  }) {
    this.choruz = params.choruz;
    this.mappings = params.mappings;
    this.bridgePrincipalId = params.bridgePrincipalId;

    this.bot = new Bot(params.botToken);
    this.registerCommands();
    this.registerHandlers();
  }

  // ── lifecycle ───────────────────────────────────────────────────────

  async start(): Promise<void> {
    // Long polling — no public URL required
    void this.bot.start({
      onStart: () => console.log('[telegram] Adapter started (long polling)'),
    });
  }

  async stop(): Promise<void> {
    this.bot.stop();
    console.log('[telegram] Adapter stopped');
  }

  // ── outbound: Choruz -> Telegram ────────────────────────────────────

  /** Push a Choruz message to the corresponding Telegram chat. */
  async pushToTelegram(
    channelId: string,
    content: string,
    senderName: string,
  ): Promise<void> {
    try {
      await this.bot.api.sendMessage(
        Number(channelId),
        `*${escapeMd(senderName)}*: ${escapeMd(content)}`,
        { parse_mode: 'MarkdownV2' },
      );
    } catch {
      // Fall back to plain text if Markdown parsing fails.
      await this.bot.api.sendMessage(Number(channelId), `${senderName}: ${content}`);
    }
  }

  // ── expose bot for slash command registration ───────────────────────

  getBot(): Bot {
    return this.bot;
  }

  // ── bot commands ─────────────────────────────────────────────────────

  private registerCommands(): void {
    // /new_agent <name> <driver> [instructions]
    // Example: /new_agent backend-dev claude Write backend Rust code
    this.bot.command('new_agent', async (ctx) => {
      const args = (ctx.match as string).trim().split(/\s+/);
      if (args.length < 2 || !args[0]) {
        await ctx.reply(
          'Usage: /new_agent <name> <driver> [instructions]\nExample: /new_agent backend-dev claude',
        );
        return;
      }

      const [name, driver, ...rest] = args;
      const instructions = rest.join(' ') || undefined;

      try {
        const result = await this.choruz.provisionAgent({
          name: name!,
          driver: driver!,
          instructions,
        });
        await ctx.reply(
          `Agent "${name}" provisioned successfully (principal: ${result.principal_id})`,
        );
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        await ctx.reply(`Failed to provision agent: ${msg}`);
      }
    });

    // /new_group <name>
    // Example: /new_group project-alpha
    this.bot.command('new_group', async (ctx) => {
      const name = (ctx.match as string).trim();
      if (!name) {
        await ctx.reply(
          'Usage: /new_group <name>\nExample: /new_group project-alpha',
        );
        return;
      }

      try {
        const chatId = String(ctx.chat.id);
        const group = await this.choruz.createGroup({
          name,
          member_ids: [this.bridgePrincipalId],
        });

        // The current database column is renamed in the Phase 3B migration.
        await this.mappings.createMapping({
          platform: 'telegram',
          platform_channel_id: chatId,
          choruz_conversation_id: group.id,
          platform_channel_name: ctx.chat.title ?? chatId,
        });

        await ctx.reply(
          `Group "${name}" created in Choruz (id: ${group.id}). This Telegram chat is now linked.`,
        );
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        await ctx.reply(`Failed to create group: ${msg}`);
      }
    });
  }

  // ── inbound: Telegram -> Choruz ─────────────────────────────────────

  private registerHandlers(): void {
    this.bot.on('message:text', async (ctx) => {
      const chatId = String(ctx.chat.id);
      const text = ctx.message.text;
      const from = ctx.from;

      // Skip messages from bots to avoid loops
      if (from?.is_bot) return;

      const senderName =
        from?.first_name ?? from?.username ?? 'unknown';
      const userId = String(from?.id ?? 'unknown');

      console.log(
        `[telegram] Message in ${chatId} from ${senderName}: ${text.slice(0, 80)}`,
      );

      try {
        // Resolve or create a Choruz conversation for this Telegram chat.
        const choruzConvId = await this.mappings.getOrCreate(
          'telegram',
          chatId,
          `tg-${ctx.chat.title ?? chatId}`,
          async () => {
            const chatTitle = ctx.chat.title ?? `tg-${chatId}`;
            const group = await this.choruz.createGroup({
              name: `tg-${chatTitle}`,
              member_ids: [this.bridgePrincipalId],
            });
            return group.id;
          },
        );

        // Convert Telegram mentions to Choruz format.
        const botUsername = this.bot.botInfo?.username ?? '';
        const choruzText = telegramMentionToChoruz(text, botUsername);

        // Forward to Choruz.
        await this.choruz.sendMessage({
          actor_id: this.bridgePrincipalId,
          conversation_id: choruzConvId,
          content: choruzText,
          metadata: {
            bridge_platform: 'telegram',
            bridge_sender: senderName,
            bridge_user_id: userId,
          },
        });
      } catch (err) {
        console.error('[telegram] Failed to forward message to Choruz:', err);
      }
    });
  }
}

/**
 * Escape special characters for Telegram MarkdownV2.
 * See: https://core.telegram.org/bots/api#markdownv2-style
 */
function escapeMd(text: string): string {
  return text.replace(/([_*\[\]()~`>#+\-=|{}.!\\])/g, '\\$1');
}
