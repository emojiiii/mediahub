CREATE TABLE s3_multipart_uploads (
    upload_id TEXT PRIMARY KEY CHECK (length(upload_id) > 0),
    application_id UUID NOT NULL REFERENCES applications(id),
    bucket_id UUID NOT NULL REFERENCES buckets(id),
    object_key TEXT NOT NULL CHECK (length(object_key) > 0),
    content_type TEXT NOT NULL CHECK (length(content_type) > 0),
    user_metadata JSONB NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(user_metadata) = 'object'),
    storage_backend TEXT NOT NULL CHECK (
        octet_length(storage_backend) BETWEEN 1 AND 255
    ),
    state TEXT NOT NULL CHECK (state IN ('pending', 'completing', 'completed', 'aborted')),
    expires_at TIMESTAMPTZ NOT NULL,
    completion_token TEXT,
    completion_lease_until TIMESTAMPTZ,
    completion_manifest JSONB,
    final_etag TEXT,
    completed_at TIMESTAMPTZ,
    aborted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT s3_multipart_expiry_check CHECK (expires_at > created_at),
    CONSTRAINT s3_multipart_completion_claim_check CHECK (
        (state = 'completing' AND completion_token IS NOT NULL
            AND completion_lease_until IS NOT NULL AND completion_manifest IS NOT NULL)
        OR (state <> 'completing' AND completion_token IS NULL
            AND completion_lease_until IS NULL)
    ),
    CONSTRAINT s3_multipart_final_etag_check CHECK (
        final_etag IS NULL OR final_etag ~ '^[0-9a-f]{32}-([1-9][0-9]{0,3}|10000)$'
    ),
    CONSTRAINT s3_multipart_completion_state_check CHECK (
        (state = 'completed' AND final_etag IS NOT NULL AND completed_at IS NOT NULL)
        OR (state <> 'completed' AND final_etag IS NULL AND completed_at IS NULL)
    ),
    CONSTRAINT s3_multipart_abort_state_check CHECK (
        (state = 'aborted' AND aborted_at IS NOT NULL)
        OR (state <> 'aborted' AND aborted_at IS NULL)
    )
);

CREATE INDEX s3_multipart_active_object_idx
    ON s3_multipart_uploads(application_id, bucket_id, object_key)
    WHERE state IN ('pending', 'completing');
CREATE INDEX s3_multipart_expiry_idx
    ON s3_multipart_uploads(expires_at, upload_id)
    WHERE state = 'pending';
CREATE INDEX s3_multipart_completion_lease_idx
    ON s3_multipart_uploads(completion_lease_until, upload_id)
    WHERE state = 'completing';

CREATE TABLE s3_multipart_parts (
    upload_id TEXT NOT NULL REFERENCES s3_multipart_uploads(upload_id) ON DELETE CASCADE,
    part_number INTEGER NOT NULL CHECK (part_number BETWEEN 1 AND 10000),
    size_bytes BIGINT NOT NULL CHECK (size_bytes >= 0),
    sha256 TEXT NOT NULL CHECK (length(sha256) = 64 AND sha256 ~ '^[0-9A-Fa-f]{64}$'),
    md5 TEXT NOT NULL CHECK (md5 ~ '^[0-9a-f]{32}$'),
    etag TEXT NOT NULL CHECK (etag ~ '^[0-9a-f]{32}$' AND etag = md5),
    storage_key TEXT NOT NULL UNIQUE CHECK (length(storage_key) > 0),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (upload_id, part_number)
);

CREATE INDEX s3_multipart_parts_order_idx
    ON s3_multipart_parts(upload_id, part_number);