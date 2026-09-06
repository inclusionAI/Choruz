CREATE TABLE online_link (id TEXT PRIMARY KEY, owner_id TEXT NOT NULL REFERENCES user(id) ON DELETE CASCADE, peer_id TEXT REFERENCES user(id) ON DELETE CASCADE, invitation_hash TEXT UNIQUE, expires_at INTEGER NOT NULL, created_at INTEGER NOT NULL);
CREATE INDEX online_link_owner ON online_link(owner_id);
CREATE INDEX online_link_peer ON online_link(peer_id);
