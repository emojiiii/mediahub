CREATE TABLE users (
    id UUID PRIMARY KEY,
    email_normalized TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    email_verified_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE applications (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    name TEXT NOT NULL,
    app_id TEXT NOT NULL UNIQUE,
    quota_bytes BIGINT NOT NULL DEFAULT 1073741824 CHECK (quota_bytes >= 0),
    used_bytes BIGINT NOT NULL DEFAULT 0 CHECK (used_bytes >= 0),
    reserved_bytes BIGINT NOT NULL DEFAULT 0 CHECK (reserved_bytes >= 0),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    CHECK (used_bytes + reserved_bytes <= quota_bytes)
);

CREATE TABLE buckets (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL REFERENCES applications(id),
    name TEXT NOT NULL,
    visibility TEXT NOT NULL CHECK (visibility IN ('public', 'private')),
    default_ttl_seconds BIGINT CHECK (default_ttl_seconds IS NULL OR default_ttl_seconds > 0),
    max_object_bytes BIGINT CHECK (max_object_bytes IS NULL OR max_object_bytes > 0),
    allowed_mime_types JSONB NOT NULL DEFAULT '[]'::jsonb,
    lifecycle_policy JSONB,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (application_id, name)
);

CREATE TABLE media (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL REFERENCES applications(id),
    bucket_id UUID NOT NULL REFERENCES buckets(id),
    object_key TEXT NOT NULL,
    original_name TEXT,
    display_name TEXT NOT NULL,
    extension TEXT,
    storage_key TEXT NOT NULL UNIQUE,
    storage_backend TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'uploading', 'active', 'archive_pending', 'archived',
        'delete_pending', 'deleted', 'quarantined'
    )),
    visibility_override TEXT CHECK (visibility_override IN ('public', 'private')),
    content_type TEXT NOT NULL,
    size_bytes BIGINT NOT NULL CHECK (size_bytes >= 0),
    sha256 TEXT NOT NULL CHECK (length(sha256) = 64),
    width INTEGER,
    height INTEGER,
    duration_ms BIGINT,
    user_metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    ai_metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    metadata_version INTEGER NOT NULL DEFAULT 1 CHECK (metadata_version > 0),
    revision BIGINT NOT NULL DEFAULT 0 CHECK (revision >= 0),
    expires_at TIMESTAMPTZ,
    archived_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (application_id, bucket_id, object_key),
    CHECK ((width IS NULL) = (height IS NULL))
);

CREATE INDEX media_bucket_created_idx
    ON media(application_id, bucket_id, created_at DESC, id DESC);
CREATE INDEX media_expiry_idx
    ON media(expires_at, id) WHERE state = 'active' AND expires_at IS NOT NULL;

CREATE TABLE variants (
    id UUID PRIMARY KEY,
    media_id UUID NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    transform_key TEXT NOT NULL,
    parameters_json TEXT NOT NULL,
    processor_version TEXT NOT NULL,
    format TEXT NOT NULL CHECK (format IN ('jpeg', 'png', 'webp', 'avif')),
    width INTEGER CHECK (width > 0),
    height INTEGER CHECK (height > 0),
    size_bytes BIGINT CHECK (size_bytes >= 0),
    storage_backend TEXT NOT NULL,
    storage_key TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK (status IN ('generating', 'ready', 'failed', 'delete_pending')),
    generation_token UUID,
    generation_lease_until TIMESTAMPTZ,
    last_error TEXT,
    last_accessed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (media_id, transform_key),
    CHECK ((generation_token IS NULL) = (generation_lease_until IS NULL)),
    CHECK (
        (status = 'generating' AND generation_token IS NOT NULL)
        OR (status <> 'generating' AND generation_token IS NULL)
    ),
    CHECK (
        (status = 'ready' AND width IS NOT NULL AND height IS NOT NULL AND size_bytes IS NOT NULL)
        OR status <> 'ready'
    )
);

CREATE INDEX variants_media_status_idx
    ON variants(media_id, status, created_at DESC, id);
CREATE INDEX variants_cleanup_idx
    ON variants(status, updated_at, id)
    WHERE status IN ('failed', 'delete_pending');

CREATE TABLE outbox_events (
    id TEXT PRIMARY KEY,
    application_id UUID NOT NULL REFERENCES applications(id),
    event_type TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    available_at TIMESTAMPTZ NOT NULL,
    leased_until TIMESTAMPTZ,
    lease_token UUID,
    delivered_at TIMESTAMPTZ,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    CHECK ((lease_token IS NULL) = (leased_until IS NULL))
);

CREATE INDEX outbox_claimable_idx
    ON outbox_events(available_at, id)
    WHERE delivered_at IS NULL;

CREATE TABLE webhook_endpoints (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL REFERENCES applications(id),
    url TEXT NOT NULL,
    secret_ciphertext TEXT NOT NULL,
    secret_key_version INTEGER NOT NULL CHECK (secret_key_version > 0),
    subscribed_events JSONB NOT NULL DEFAULT '[]'::jsonb,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX webhook_endpoints_application_idx
    ON webhook_endpoints(application_id, created_at DESC, id DESC);

CREATE TABLE webhook_deliveries (
    event_id TEXT NOT NULL REFERENCES outbox_events(id) ON DELETE CASCADE,
    endpoint_id UUID NOT NULL REFERENCES webhook_endpoints(id) ON DELETE CASCADE,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ,
    leased_until TIMESTAMPTZ,
    lease_token UUID,
    delivered_at TIMESTAMPTZ,
    dead_lettered_at TIMESTAMPTZ,
    last_error TEXT,
    last_response_status INTEGER CHECK (
        last_response_status IS NULL OR last_response_status BETWEEN 100 AND 599
    ),
    replay_count INTEGER NOT NULL DEFAULT 0 CHECK (replay_count >= 0),
    last_replayed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (event_id, endpoint_id),
    CHECK ((lease_token IS NULL) = (leased_until IS NULL)),
    CHECK (delivered_at IS NULL OR dead_lettered_at IS NULL)
);

CREATE INDEX webhook_deliveries_claimable_idx
    ON webhook_deliveries(next_attempt_at, event_id, endpoint_id)
    WHERE delivered_at IS NULL AND dead_lettered_at IS NULL;

CREATE INDEX webhook_deliveries_history_idx
    ON webhook_deliveries(endpoint_id, updated_at DESC, event_id DESC);

CREATE TABLE upload_sessions (
    id UUID PRIMARY KEY,
    media_id UUID NOT NULL UNIQUE,
    application_id UUID NOT NULL REFERENCES applications(id),
    bucket_id UUID NOT NULL REFERENCES buckets(id),
    object_key TEXT NOT NULL,
    original_name TEXT,
    display_name TEXT NOT NULL,
    extension TEXT,
    expected_size_bytes BIGINT NOT NULL CHECK (expected_size_bytes >= 0),
    expected_mime TEXT NOT NULL,
    storage_backend TEXT NOT NULL,
    storage_key TEXT NOT NULL UNIQUE,
    visibility_override TEXT CHECK (visibility_override IN ('public', 'private')),
    media_expires_at TIMESTAMPTZ,
    user_metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    ai_metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    session_expires_at TIMESTAMPTZ NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'completed', 'cancelled', 'expired')),
    completed_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    expired_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX upload_sessions_pending_object_idx
    ON upload_sessions(application_id, bucket_id, object_key)
    WHERE state = 'pending';
CREATE INDEX upload_sessions_expiry_idx
    ON upload_sessions(session_expires_at, id) WHERE state = 'pending';

CREATE TABLE async_jobs (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL REFERENCES applications(id),
    operation_scope TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL CHECK (length(request_hash) = 64),
    request_id TEXT,
    action_type TEXT NOT NULL CHECK (
        action_type IN ('update_ttl_seconds', 'update_visibility', 'delete')
    ),
    action_payload JSONB NOT NULL,
    state TEXT NOT NULL CHECK (
        state IN ('pending', 'running', 'completed', 'failed', 'cancelled')
    ),
    total_items INTEGER NOT NULL CHECK (total_items > 0),
    succeeded_items INTEGER NOT NULL DEFAULT 0 CHECK (succeeded_items >= 0),
    failed_items INTEGER NOT NULL DEFAULT 0 CHECK (failed_items >= 0),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    max_attempts INTEGER NOT NULL CHECK (max_attempts > 0),
    next_attempt_at TIMESTAMPTZ,
    lease_token UUID,
    leased_until TIMESTAMPTZ,
    error_summary TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    failed_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (id, application_id),
    UNIQUE (application_id, operation_scope, idempotency_key),
    CHECK (succeeded_items + failed_items <= total_items),
    CHECK ((lease_token IS NULL) = (leased_until IS NULL))
);

CREATE INDEX async_jobs_application_idx
    ON async_jobs(application_id, created_at DESC, id DESC);
CREATE INDEX async_jobs_claimable_idx
    ON async_jobs(next_attempt_at, leased_until, created_at, id)
    WHERE state IN ('pending', 'running');

CREATE TABLE async_job_item_results (
    job_id UUID NOT NULL,
    application_id UUID NOT NULL,
    media_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    state TEXT NOT NULL DEFAULT 'pending' CHECK (
        state IN ('pending', 'succeeded', 'failed', 'cancelled')
    ),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    result JSONB,
    error_code TEXT,
    error_summary TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (job_id, media_id),
    UNIQUE (job_id, ordinal),
    FOREIGN KEY (job_id, application_id)
        REFERENCES async_jobs(id, application_id) ON DELETE CASCADE
);

CREATE INDEX async_job_items_application_idx
    ON async_job_item_results(application_id, job_id, ordinal);
