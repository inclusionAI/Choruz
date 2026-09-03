CREATE TABLE IF NOT EXISTS moment_post (
    id TEXT PRIMARY KEY,
    author_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    content TEXT NOT NULL DEFAULT '',
    image_attachment_id TEXT REFERENCES attachment(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_moment_post_created_at
    ON moment_post (created_at DESC);

CREATE TABLE IF NOT EXISTS moment_post_like (
    post_id TEXT NOT NULL REFERENCES moment_post(id) ON DELETE CASCADE,
    principal_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (post_id, principal_id)
);

CREATE TABLE IF NOT EXISTS moment_comment (
    id TEXT PRIMARY KEY,
    post_id TEXT NOT NULL REFERENCES moment_post(id) ON DELETE CASCADE,
    author_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_moment_comment_post_created_at
    ON moment_comment (post_id, created_at);

CREATE TABLE IF NOT EXISTS moment_comment_like (
    comment_id TEXT NOT NULL REFERENCES moment_comment(id) ON DELETE CASCADE,
    principal_id TEXT NOT NULL REFERENCES principal(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (comment_id, principal_id)
);
