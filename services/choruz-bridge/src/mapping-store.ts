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

  constructor(connection: string | pg.PoolConfig) {
    this.pool = new pg.Pool(typeof connection === 'string' ? { connectionString: connection } : connection);
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
   * channel. A database-scoped channel lock serializes factories across bridge
   * processes. Existing mappings are reused; failures never announce a new link.
   */
  async getOrCreate(
    platform: string,
    platformChannelId: string,
    channelName: string,
    createFn: () => Promise<string>,
  ): Promise<string> {
    const client = await this.pool.connect();
    try {
      await client.query('BEGIN');
      await client.query('SELECT pg_advisory_xact_lock(hashtextextended($1, 0))', [
        JSON.stringify([platform, platformChannelId]),
      ]);
      const existing = await client.query(
        `SELECT choruz_conversation_id FROM bridge_channel_mappings
         WHERE platform = $1 AND platform_channel_id = $2`,
        [platform, platformChannelId],
      );
      const conversation = existing.rows.length
        ? (existing.rows[0] as { choruz_conversation_id: string }).choruz_conversation_id
        : await createFn();
      if (!existing.rows.length) {
        await client.query(
          `INSERT INTO bridge_channel_mappings
           (platform, platform_channel_id, choruz_conversation_id, platform_channel_name)
           VALUES ($1, $2, $3, $4)`,
          [platform, platformChannelId, conversation, channelName],
        );
      }
      await client.query('COMMIT');
      return conversation;
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  }

  async close(): Promise<void> {
    await this.pool.end();
  }
}
