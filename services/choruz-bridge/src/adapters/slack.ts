import { App } from '@slack/bolt';
import type { ChoruzClient } from '../choruz-client.js';
import type { MappingStore } from '../mapping-store.js';
import { slackMentionToChoruz } from '../mention.js';

/** Minimal shape of a regular Slack message event. */
interface SlackMessage {
  channel: string;
  user?: string;
  text?: string;
  bot_id?: string;
  subtype?: string;
}

export class SlackAdapter {
  private app: App;
  private choruz: ChoruzClient;
  private mappings: MappingStore;
  private bridgePrincipalId: string;

  /** Slack user ID -> Choruz agent name. Populated lazily. */
  private agentNameMap = new Map<string, string>();

  constructor(params: {
    botToken: string;
    appToken: string;
    choruz: ChoruzClient;
    mappings: MappingStore;
    bridgePrincipalId: string;
  }) {
    this.choruz = params.choruz;
    this.mappings = params.mappings;
    this.bridgePrincipalId = params.bridgePrincipalId;

    this.app = new App({
      token: params.botToken,
      appToken: params.appToken,
      socketMode: true,
    });

    this.registerMessageHandler();
    this.registerSlashCommands();
  }

  // ── lifecycle ───────────────────────────────────────────────────────

  async start(): Promise<void> {
    await this.app.start();
    console.log('[slack] Adapter started (Socket Mode)');
  }

  async stop(): Promise<void> {
    await this.app.stop();
    console.log('[slack] Adapter stopped');
  }

  // ── outbound: Choruz -> Slack ───────────────────────────────────────

  /** Push a Choruz message to the corresponding Slack channel. */
  async pushToSlack(
    channelId: string,
    content: string,
    senderName: string,
  ): Promise<void> {
    await this.app.client.chat.postMessage({
      channel: channelId,
      text: `*${senderName}*: ${content}`,
    });
  }

  // ── slash commands ───────────────────────────────────────────────────

  /** Expose the bolt App for external use. */
  getApp(): App {
    return this.app;
  }

  private registerSlashCommands(): void {
    // /new-agent <name> <driver>
    // Example: /new-agent backend-dev claude
    this.app.command('/new-agent', async ({ command, ack, respond }) => {
      await ack();

      const args = command.text.trim().split(/\s+/);
      if (args.length < 2) {
        await respond({
          text: 'Usage: `/new-agent <name> <driver>`\nExample: `/new-agent backend-dev claude`',
          response_type: 'ephemeral',
        });
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
        await respond({
          text: `Agent *${name}* provisioned successfully (principal: \`${result.principal_id}\`)`,
          response_type: 'in_channel',
        });
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        await respond({
          text: `Failed to provision agent: ${msg}`,
          response_type: 'ephemeral',
        });
      }
    });

    // /new-group <name>
    // Example: /new-group project-alpha
    this.app.command('/new-group', async ({ command, ack, respond }) => {
      await ack();

      const name = command.text.trim();
      if (!name) {
        await respond({
          text: 'Usage: `/new-group <name>`\nExample: `/new-group project-alpha`',
          response_type: 'ephemeral',
        });
        return;
      }

      try {
        const group = await this.choruz.createGroup({
          name,
          member_ids: [this.bridgePrincipalId],
        });

        // The current database column is renamed in the Phase 3B migration.
        await this.mappings.createMapping({
          platform: 'slack',
          platform_channel_id: command.channel_id,
          choruz_conversation_id: group.id,
          platform_channel_name: command.channel_name,
        });

        await respond({
          text: `Group *${name}* created in Choruz (id: \`${group.id}\`). This Slack channel is now linked.`,
          response_type: 'in_channel',
        });
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        await respond({
          text: `Failed to create group: ${msg}`,
          response_type: 'ephemeral',
        });
      }
    });
  }

  // ── inbound: Slack -> Choruz ─────────────────────────────────────────

  private registerMessageHandler(): void {
    this.app.message(async ({ message }) => {
      // Only handle regular user messages (not bot messages, not edits, etc.)
      const msg = message as SlackMessage;

      // Skip bot messages to avoid loops
      if ('bot_id' in msg && msg.bot_id) return;
      // Skip messages without text
      if (!msg.text) return;

      const channelId = msg.channel;
      const userId = msg.user ?? 'unknown';
      const text = msg.text;

      console.log(
        `[slack] Message in ${channelId} from ${userId}: ${text.slice(0, 80)}`,
      );

      try {
        // Resolve or create a Choruz conversation for this Slack channel.
        const choruzConvId = await this.mappings.getOrCreate(
          'slack',
          channelId,
          `slack-${channelId}`,
          async () => {
            // Fetch channel info for a better name
            let channelName = channelId;
            try {
              const info = await this.app.client.conversations.info({
                channel: channelId,
              });
              channelName = (info.channel as { name?: string })?.name ?? channelId;
            } catch {
              // Fall back to channel ID
            }

            const group = await this.choruz.createGroup({
              name: `slack-${channelName}`,
              member_ids: [this.bridgePrincipalId],
            });
            return group.id;
          },
        );

        // Convert Slack mentions to Choruz mentions.
        const choruzText = slackMentionToChoruz(
          text,
          '', // bot user ID not needed for V1
          this.agentNameMap,
        );

        // Forward to Choruz.
        await this.choruz.sendMessage({
          actor_id: this.bridgePrincipalId,
          conversation_id: choruzConvId,
          content: choruzText,
          metadata: {
            bridge_platform: 'slack',
            bridge_sender: userId,
            bridge_user_id: userId,
          },
        });
      } catch (err) {
        console.error('[slack] Failed to forward message to Choruz:', err);
      }
    });
  }
}
