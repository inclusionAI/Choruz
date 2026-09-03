-- Bridge channel mappings: maps external platform channels to echat conversations
CREATE TABLE IF NOT EXISTS bridge_channel_mappings (
    platform TEXT NOT NULL,             -- 'slack' | 'telegram'
    platform_channel_id TEXT NOT NULL,  -- Slack channel ID (C...) / Telegram chat_id
    echat_conversation_id TEXT NOT NULL, -- echat group conversation UUID
    platform_channel_name TEXT,          -- human-readable channel name for logging
    created_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (platform, platform_channel_id)
);

CREATE INDEX IF NOT EXISTS idx_bridge_mappings_echat_conv
    ON bridge_channel_mappings (echat_conversation_id);
