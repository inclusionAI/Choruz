import pg from 'pg';

export interface ChannelMapping {
  platform: string;
  platform_channel_id: string;
  platform_channel_name: string | null;
}

/**
 * Reads and writes bridge_channel_mappings in PostgreSQL.
 *
 * The key method is `getOrCreate()`: on the first message from a new
 * platform channel it calls the provided factory to create a Choruz
 * conversation, records the mapping, and returns the conversation ID.
 * Subsequent calls return the cached mapping directly.
 */
export class MappingStore {
  private pool: pg.Pool;

  constructor(connectionString: string) {
    this.pool = new pg.Pool({ connectionString });
  }

  /** Look up a Choruz conversation ID for a platform channel. Returns null if not mapped. */
  async findMapping(
    platform: string,
    platformChannelId: string,
  ): Promise<string | null> {
    const { rows } = await this.pool.query(
      `SELECT choruz_conversation_id FROM bridge_channel_mappings
       WHERE platform = $1 AND platform_channel_id = $2`,
      [platform, platformChannelId],
    );
    if (rows.length === 0) return null;
    return (rows[0] as { choruz_conversation_id: string }).choruz_conversation_id;
  }

  /** Create a new mapping. */
  async createMapping(params: {
    platform: string;
    platform_channel_id: string;
    choruz_conversation_id: string;
    platform_channel_name?: string;
  }): Promise<void> {
    await this.pool.query(
      `INSERT INTO bridge_channel_mappings
         (platform, platform_channel_id, choruz_conversation_id, platform_channel_name)
       VALUES ($1, $2, $3, $4)
       ON CONFLICT (platform, platform_channel_id) DO NOTHING`,
      [
        params.platform,
        params.platform_channel_id,
        params.choruz_conversation_id,
        params.platform_channel_name ?? null,
      ],
    );
  }

  /** Reverse-lookup: find all platform channels mapped to a Choruz conversation. */
  async findByChoruzConversation(
    choruzConversationId: string,
  ): Promise<ChannelMapping[]> {
    const { rows } = await this.pool.query(
      `SELECT platform, platform_channel_id, platform_channel_name
       FROM bridge_channel_mappings
       WHERE choruz_conversation_id = $1`,
      [choruzConversationId],
    );
    return rows as ChannelMapping[];
  }

  /**
   * Get-or-create: returns the Choruz conversation ID for the given platform
   * channel. If no mapping exists yet, calls `createFn` to provision a Choruz
   * conversation and records the mapping before returning.
   */
  async getOrCreate(
    platform: string,
    platformChannelId: string,
    channelName: string,
    createFn: () => Promise<string>,
  ): Promise<string> {
    const existing = await this.findMapping(platform, platformChannelId);
    if (existing) return existing;

    const choruzConversationId = await createFn();

    await this.createMapping({
      platform,
      platform_channel_id: platformChannelId,
      choruz_conversation_id: choruzConversationId,
      platform_channel_name: channelName,
    });

    return choruzConversationId;
  }

  async close(): Promise<void> {
    await this.pool.end();
  }
}
