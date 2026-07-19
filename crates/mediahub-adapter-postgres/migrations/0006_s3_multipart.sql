CREATE TABLE s3_multipart_uploads (
    upload_id TEXT PRIMARY KEY CHECK (length(upload_id) > 0),
    application_id UUID NOT NULL REFERENCES applications(id),
    bucket_id UUID NOT NULL REFERENCES buckets(id),
    object_key TEXT NOT NULL CHECK (length(object_key) > 0),
    content_type TEXT NOT NULL CHECK (length(content_type) > 0),
    visibility_override TEXT CHECK (visibility_override IN ('public', 'private')),
    state TEXT NOT NULL CHECK (state IN ('pending', 'completing', 'completed', 'aborted')),
    expires_at TIMESTAMPTZ NOT NULL,
    completion_token TEXT,
    completion_lease_until TIMESTAMPTZ,
    completion_manifest JSONB,
    media_id UUID REFERENCES media(id),
    final_etag TEXT,
    completed_at TIMESTAMPTZ,
    aborted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    CHECK (expires_at > created_at),
    CHECK (
        (state = 'completing' AND completion_token IS NOT NULL
            AND completion_lease_until IS NOT NULL AND completion_manifest IS NOT NULL)
        OR (state <> 'completing' AND completion_token IS NULL
            AND completion_lease_until IS NULL)
    ),
    CHECK (
        (state = 'completed' AND media_id IS NOT NULL AND final_etag IS NOT NULL
            AND completed_at IS NOT NULL)
        OR state <> 'completed'
    )
);

CREATE UNIQUE INDEX s3_multipart_active_object_idx
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
    etag TEXT NOT NULL CHECK (length(etag) > 0),
    storage_key TEXT NOT NULL UNIQUE CHECK (length(storage_key) > 0),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (upload_id, part_number)
);

CREATE INDEX s3_multipart_parts_order_idx
    ON s3_multipart_parts(upload_id, part_number);
