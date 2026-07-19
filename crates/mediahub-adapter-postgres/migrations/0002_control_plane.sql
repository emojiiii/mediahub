-- Control-plane schema required before the PostgreSQL adapter can back the
-- complete server. Repository methods are intentionally enabled only after
-- their behavior is covered by the same contracts as SQLite.

ALTER TABLE users DROP CONSTRAINT IF EXISTS users_status_check;
ALTER TABLE users
    ADD CONSTRAINT users_status_check CHECK (
        status IN ('pending_verification', 'active', 'suspended', 'deleted')
    );
ALTER TABLE users
    ADD COLUMN system_role TEXT NOT NULL DEFAULT 'user'
        CHECK (system_role IN ('user', 'admin'));
ALTER TABLE users ADD COLUMN last_login_at TIMESTAMPTZ;

CREATE INDEX users_role_status_created_idx
    ON users(system_role, status, created_at DESC, id DESC);

CREATE INDEX applications_user_created_idx
    ON applications(user_id, created_at DESC, id DESC);

CREATE TABLE sessions (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    token_hash TEXT NOT NULL UNIQUE,
    csrf_token_hash TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    last_seen_at TIMESTAMPTZ NOT NULL,
    created_ip TEXT,
    last_seen_ip TEXT,
    user_agent_summary TEXT,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX sessions_user_expiry_idx
    ON sessions(user_id, expires_at DESC);
CREATE INDEX sessions_active_user_idx
    ON sessions(user_id, revoked_at, expires_at DESC);

CREATE TABLE one_time_tokens (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    purpose TEXT NOT NULL CHECK (purpose IN ('verify_email', 'reset_password')),
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX one_time_tokens_user_purpose_idx
    ON one_time_tokens(user_id, purpose, consumed_at, expires_at);
CREATE INDEX one_time_tokens_expiry_idx
    ON one_time_tokens(expires_at) WHERE consumed_at IS NULL;

CREATE TABLE access_keys (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL REFERENCES applications(id),
    access_key_id TEXT NOT NULL UNIQUE,
    secret_ciphertext TEXT NOT NULL,
    secret_key_version INTEGER NOT NULL CHECK (secret_key_version > 0),
    secret_last_four TEXT NOT NULL,
    name TEXT NOT NULL,
    permissions JSONB NOT NULL DEFAULT '[]'::jsonb
        CHECK (jsonb_typeof(permissions) = 'array'),
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX access_keys_application_idx
    ON access_keys(application_id, created_at DESC, id DESC);

CREATE TABLE replay_nonces (
    access_key_id TEXT NOT NULL REFERENCES access_keys(access_key_id) ON DELETE CASCADE,
    nonce TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (access_key_id, nonce)
);

CREATE INDEX replay_nonces_expiry_idx ON replay_nonces(expires_at);

CREATE TABLE idempotency_keys (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL REFERENCES applications(id),
    operation_scope TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL CHECK (length(request_hash) = 64),
    status TEXT NOT NULL CHECK (status IN ('in_progress', 'completed')),
    response_status INTEGER CHECK (
        response_status IS NULL OR response_status BETWEEN 100 AND 599
    ),
    response_payload TEXT,
    resource_id TEXT,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ,
    UNIQUE (application_id, operation_scope, idempotency_key),
    CHECK (
        (status = 'in_progress'
            AND response_status IS NULL
            AND response_payload IS NULL
            AND completed_at IS NULL)
        OR (status = 'completed'
            AND response_status IS NOT NULL
            AND response_payload IS NOT NULL
            AND completed_at IS NOT NULL)
    )
);

CREATE INDEX idempotency_keys_expiry_idx ON idempotency_keys(expires_at);

CREATE TABLE audit_logs (
    id TEXT PRIMARY KEY,
    application_id UUID NOT NULL REFERENCES applications(id),
    actor_type TEXT NOT NULL CHECK (actor_type IN ('user', 'access_key', 'system')),
    actor_id TEXT NOT NULL,
    action TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    summary JSONB NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(summary) = 'object'),
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX audit_logs_application_created_idx
    ON audit_logs(application_id, created_at DESC, id DESC);

CREATE TABLE deployment_bootstrap (
    bootstrap_key TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    completed_at TIMESTAMPTZ NOT NULL
);
