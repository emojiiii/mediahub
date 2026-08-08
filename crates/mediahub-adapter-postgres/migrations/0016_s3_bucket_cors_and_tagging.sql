-- Independent, revisioned S3 Bucket CORS and Bucket Tagging subresources.
-- A positive revision with a NULL document records a successful DELETE and
-- distinguishes it from a bucket that has never received the mutation.

ALTER TABLE buckets
    ADD COLUMN s3_cors_configuration JSONB,
    ADD COLUMN s3_cors_revision BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN s3_cors_updated_at TIMESTAMPTZ,
    ADD COLUMN s3_bucket_tags JSONB,
    ADD COLUMN s3_bucket_tags_revision BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN s3_bucket_tags_updated_at TIMESTAMPTZ,
    ADD CONSTRAINT buckets_s3_cors_document_check CHECK (
        s3_cors_configuration IS NULL
        OR jsonb_typeof(s3_cors_configuration) = 'object'
    ),
    ADD CONSTRAINT buckets_s3_cors_revision_check CHECK (
        s3_cors_revision >= 0
        AND (
            (
                s3_cors_revision = 0
                AND s3_cors_configuration IS NULL
                AND s3_cors_updated_at IS NULL
            )
            OR (s3_cors_revision > 0 AND s3_cors_updated_at IS NOT NULL)
        )
    ),
    ADD CONSTRAINT buckets_s3_bucket_tags_document_check CHECK (
        s3_bucket_tags IS NULL
        OR jsonb_typeof(s3_bucket_tags) = 'array'
    ),
    ADD CONSTRAINT buckets_s3_bucket_tags_revision_check CHECK (
        s3_bucket_tags_revision >= 0
        AND (
            (
                s3_bucket_tags_revision = 0
                AND s3_bucket_tags IS NULL
                AND s3_bucket_tags_updated_at IS NULL
            )
            OR (s3_bucket_tags_revision > 0 AND s3_bucket_tags_updated_at IS NOT NULL)
        )
    );
