CREATE TABLE short_links (
    code TEXT PRIMARY KEY CHECK (code ~ '^[A-Za-z0-9_-]{8,32}$'),
    application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    media_id UUID NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    target_path TEXT NOT NULL CHECK (target_path LIKE '/%'),
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (application_id, media_id)
);

CREATE INDEX short_links_expiry_idx
    ON short_links(expires_at, code) WHERE expires_at IS NOT NULL;

